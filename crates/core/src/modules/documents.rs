//! Local evidence vault (MDR §5.4): encrypted files, on-device extraction,
//! redacted local search, bounded excerpts for agents, explicit deletion.

use super::{IdParam, PageParams, Pager, parse_params};
use crate::ctx::Ctx;
use crate::op;
use crate::ops::OpSpec;
use crate::sql::{exec, fts_query, query_all, query_one, require, scalar};
use openclaim_domain::{OcError, OcResult};
use openclaim_extract::{DocType, MAX_IMPORT_BYTES};
use openclaim_security::digest::sha256_hex;
use openclaim_security::redact::redact_text;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocMeta {
    pub original_name: String,
    pub original_path: String,
    pub imported_by: String,
    #[serde(default)]
    pub classification: Option<String>,
    #[serde(default)]
    pub suggestions: Vec<openclaim_extract::FactSuggestion>,
    #[serde(default)]
    pub extraction_note: Option<String>,
    #[serde(default)]
    pub text_chars: usize,
}

pub fn meta_of(ctx: &Ctx<'_>, row: &Value) -> OcResult<DocMeta> {
    let enc = row["encrypted_metadata"].as_str().unwrap_or("");
    let json = ctx.svc.vault().decrypt_str(enc).map_err(|e| OcError::internal(e.to_string()))?;
    Ok(serde_json::from_str(&json)?)
}

pub fn vault_file(ctx: &Ctx<'_>, id: &str) -> PathBuf {
    ctx.svc.inner.paths.vault_dir.join(format!("{id}.bin"))
}

#[derive(Deserialize, JsonSchema)]
pub struct ImportParams {
    /// Local file path (PDF, PNG/JPG/TIFF, CSV, EML, MBOX, TXT).
    pub file: String,
    #[serde(default)]
    pub person_id: Option<String>,
}

fn safe_source_path(p: &str) -> OcResult<PathBuf> {
    let path = Path::new(p);
    if !path.is_absolute() {
        return Err(OcError::invalid("file must be an absolute path"));
    }
    let canon = std::fs::canonicalize(path).map_err(|e| OcError::invalid(format!("cannot read '{p}': {e}")))?;
    if !canon.is_file() {
        return Err(OcError::invalid("file is not a regular file"));
    }
    Ok(canon)
}

fn op_import(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ImportParams = parse_params(p)?;
    if let Some(pid) = &p.person_id {
        crate::modules::people::ensure_person(ctx, pid)?;
    }
    let path = safe_source_path(&p.file)?;
    let size = std::fs::metadata(&path).map_err(|e| OcError::invalid(e.to_string()))?.len();
    if size > MAX_IMPORT_BYTES {
        return Err(OcError::invalid(format!("file is {size} bytes; the limit is {MAX_IMPORT_BYTES}")));
    }
    let bytes = std::fs::read(&path).map_err(|e| OcError::invalid(format!("read failed: {e}")))?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let doc_type = openclaim_extract::detect(&bytes, &name)?;
    let checksum = sha256_hex(&bytes);
    if let Some(existing) = scalar::<String, _>(ctx.conn, "SELECT id FROM documents WHERE checksum = ?1", [&checksum])? {
        return Ok(json!({"id": existing, "duplicate": true, "note": "an identical file is already in the vault"}));
    }
    let id = openclaim_db::new_id();
    let meta = DocMeta { original_name: name.clone(), original_path: path.to_string_lossy().to_string(), imported_by: ctx.actor_id(), ..Default::default() };
    let vault_path = vault_file(ctx, &id);
    if !ctx.dry_run {
        let blob = ctx.svc.vault().encrypt(&bytes);
        std::fs::write(&vault_path, blob).map_err(|e| OcError::internal(format!("write vault file: {e}")))?;
    }
    exec(
        ctx.conn,
        "INSERT INTO documents(id, vault_path, checksum, doc_type, mime, size_bytes, encrypted_metadata, extraction_state, person_id, imported_at) VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9)",
        (&id, vault_path.to_string_lossy().to_string(), &checksum, doc_type.as_str(), doc_type.mime(&openclaim_extract::extension(&name)), size as i64, ctx.svc.vault().encrypt_str(&serde_json::to_string(&meta)?), &p.person_id, &ctx.now),
    )?;
    if let Some(pid) = &p.person_id {
        exec(ctx.conn, "INSERT OR IGNORE INTO document_links(document_id, target_kind, target_id, locator, validated, created_at) VALUES (?1,'person',?2,NULL,1,?3)", (&id, pid, &ctx.now))?;
    }
    let job_id = crate::jobs::enqueue(ctx, "document_extract", json!({"document_id": id}), None)?;
    ctx.audit("documents.import", &id, json!({"doc_type": doc_type.as_str(), "size": size}))?;
    Ok(json!({"id": id, "doc_type": doc_type.as_str(), "size_bytes": size, "checksum": checksum, "extraction_job_id": job_id, "duplicate": false}))
}

/// Runs inside the extraction job (outside the DB lock for the heavy part).
pub fn extract_bytes(svc: &crate::Service, id: &str) -> OcResult<(openclaim_extract::Extraction, bool)> {
    let (vault_path, doc_type, ocr, index) = {
        let guard = svc.inner.db.lock().map_err(|_| OcError::internal("db lock"))?;
        let row = require(query_one(&guard.conn, "SELECT vault_path, doc_type FROM documents WHERE id = ?1", [id])?, "document", id)?;
        let ocr = crate::modules::config::get_bool(&guard.conn, "vault.ocr");
        let index = crate::modules::config::get_bool(&guard.conn, "vault.index_text");
        (row["vault_path"].as_str().unwrap_or("").to_string(), row["doc_type"].as_str().unwrap_or("text").to_string(), ocr, index)
    };
    let blob = std::fs::read(&vault_path).map_err(|e| OcError::internal(format!("read vault file: {e}")))?;
    let bytes = svc.vault().decrypt(&blob).map_err(|e| OcError::internal(e.to_string()))?;
    let dt = match doc_type.as_str() {
        "pdf" => DocType::Pdf,
        "image" => DocType::Image,
        "csv" => DocType::Csv,
        "eml" => DocType::Eml,
        "mbox" => DocType::Mbox,
        _ => DocType::Text,
    };
    Ok((openclaim_extract::extract(&bytes, dt, &openclaim_extract::OcrConfig { tesseract: None, enabled: ocr }), index))
}

pub fn store_extraction(ctx: &mut Ctx<'_>, id: &str, ex: &openclaim_extract::Extraction, index: bool) -> OcResult<()> {
    let row = require(query_one(ctx.conn, "SELECT * FROM documents WHERE id = ?1", [id])?, "document", id)?;
    let mut meta = meta_of(ctx, &row)?;
    meta.classification = Some(ex.classification.clone());
    meta.suggestions = ex.suggestions.clone();
    meta.extraction_note = ex.note.clone();
    meta.text_chars = ex.text.chars().count();
    exec(ctx.conn, "UPDATE documents SET extraction_state = ?2, extraction_note = ?3, encrypted_metadata = ?4, version = version + 1 WHERE id = ?1", (id, &ex.state, &ex.note, ctx.svc.vault().encrypt_str(&serde_json::to_string(&meta)?)))?;
    exec(ctx.conn, "DELETE FROM documents_fts WHERE document_id = ?1", [id])?;
    if index && !ex.text.is_empty() {
        exec(ctx.conn, "INSERT INTO documents_fts(document_id, body) VALUES (?1, ?2)", (id, redact_text(&ex.text)))?;
    }
    ctx.emit("document.extracted", json!({"document_id": id, "state": ex.state, "classification": ex.classification}))?;
    Ok(())
}

fn view(ctx: &Ctx<'_>, row: Value) -> OcResult<Value> {
    let id = row["id"].as_str().unwrap_or("").to_string();
    let meta = meta_of(ctx, &row)?;
    let links = query_all(ctx.conn, "SELECT target_kind, target_id, locator, validated FROM document_links WHERE document_id = ?1", [&id])?;
    let name = if ctx.actor.is_owner() { json!(meta.original_name) } else { json!(format!("document {}", &id[..8])) };
    Ok(json!({
        "id": id,
        "name": name,
        "doc_type": row["doc_type"],
        "mime": row["mime"],
        "size_bytes": row["size_bytes"],
        "classification": meta.classification,
        "extraction_state": row["extraction_state"],
        "extraction_note": row["extraction_note"],
        "text_chars": meta.text_chars,
        "suggestion_count": meta.suggestions.len(),
        "person_id": row["person_id"],
        "links": links,
        "imported_at": row["imported_at"],
        "version": row["version"],
    }))
}

fn op_list(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    #[derive(Deserialize, JsonSchema)]
    struct P {
        #[serde(default)]
        person_id: Option<String>,
        #[serde(flatten)]
        page: PageParams,
    }
    let p: P = parse_params(p)?;
    let pager = Pager::from(&p.page, &ctx.now)?;
    let rows = match &p.person_id {
        Some(pid) => query_all(ctx.conn, &format!("SELECT * FROM documents WHERE person_id = ?1 AND imported_at <= ?2 ORDER BY imported_at DESC{}", pager.sql_limit()), (pid, &pager.snapshot))?,
        None => query_all(ctx.conn, &format!("SELECT * FROM documents WHERE imported_at <= ?1 ORDER BY imported_at DESC{}", pager.sql_limit()), [&pager.snapshot])?,
    };
    let items = rows.into_iter().map(|r| view(ctx, r)).collect::<OcResult<Vec<_>>>()?;
    Ok(pager.wrap(items, None))
}

fn op_show(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let row = require(query_one(ctx.conn, "SELECT * FROM documents WHERE id = ?1", [&p.id])?, "document", &p.id)?;
    view(ctx, row)
}

fn op_extract(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    require(scalar::<String, _>(ctx.conn, "SELECT id FROM documents WHERE id = ?1", [&p.id])?, "document", &p.id)?;
    let job_id = crate::jobs::enqueue(ctx, "document_extract", json!({"document_id": p.id}), None)?;
    Ok(json!({"document_id": p.id, "job_id": job_id, "status": "queued"}))
}

fn op_suggestions(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let row = require(query_one(ctx.conn, "SELECT * FROM documents WHERE id = ?1", [&p.id])?, "document", &p.id)?;
    let meta = meta_of(ctx, &row)?;
    ctx.disclose("document", &p.id, "suggestions")?;
    let suggestions: Vec<Value> = meta
        .suggestions
        .iter()
        .map(|s| json!({"kind": s.kind, "value": if ctx.actor.is_owner() { s.value.clone() } else { redact_text(&s.value) }, "span_start": s.span_start, "span_end": s.span_end, "excerpt": if ctx.actor.is_owner() { s.excerpt.clone() } else { redact_text(&s.excerpt) }}))
        .collect();
    Ok(json!({"document_id": p.id, "classification": meta.classification, "suggestions": suggestions, "trust": "deterministic_evaluation", "note": "pattern suggestions with source spans; confirm as facts only if true"}))
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchParams {
    pub q: String,
    #[serde(flatten)]
    pub page: PageParams,
}

fn op_search(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: SearchParams = parse_params(p)?;
    let pager = Pager::from(&p.page, &ctx.now)?;
    let fq = fts_query(&p.q);
    if fq.is_empty() {
        return Err(OcError::invalid("query needs at least one word of 3+ characters"));
    }
    let rows = query_all(
        ctx.conn,
        &format!("SELECT d.id, d.doc_type, d.extraction_state, d.person_id, d.imported_at, snippet(documents_fts, 1, '[', ']', '…', 12) AS snippet FROM documents_fts f JOIN documents d ON d.id = f.document_id WHERE documents_fts MATCH ?1 AND d.imported_at <= ?2 ORDER BY rank{}", pager.sql_limit()),
        (&fq, &pager.snapshot),
    )?;
    let mut items = Vec::new();
    for mut r in rows {
        let id = r["id"].as_str().unwrap_or("").to_string();
        if !ctx.actor.has_scope("documents.read", Some(&id)) {
            r["snippet"] = json!("[requires documents.read]");
        } else {
            ctx.disclose("document", &id, "snippet")?;
        }
        items.push(r);
    }
    Ok(pager.wrap(items, None))
}

#[derive(Deserialize, JsonSchema)]
pub struct ExcerptParams {
    pub id: String,
    #[serde(default)]
    pub offset: usize,
    /// Characters to return (max 4000).
    #[serde(default)]
    pub length: Option<usize>,
}

fn op_excerpt(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ExcerptParams = parse_params(p)?;
    require(scalar::<String, _>(ctx.conn, "SELECT id FROM documents WHERE id = ?1", [&p.id])?, "document", &p.id)?;
    let body: Option<String> = scalar(ctx.conn, "SELECT body FROM documents_fts WHERE document_id = ?1", [&p.id])?;
    let Some(body) = body else {
        return Ok(json!({"document_id": p.id, "text": null, "note": "no extracted text (extraction pending, empty, or indexing disabled)"}));
    };
    let len = p.length.unwrap_or(2000).min(4000);
    let chars: Vec<char> = body.chars().collect();
    let start = p.offset.min(chars.len());
    let end = (start + len).min(chars.len());
    let text: String = chars[start..end].iter().collect();
    ctx.disclose("document", &p.id, &format!("excerpt {start}-{end}"))?;
    Ok(json!({"document_id": p.id, "offset": start, "end": end, "total_chars": chars.len(), "text": openclaim_security::sanitize::untrusted(&text, &format!("vault document {}", p.id), 4000), "redacted": true}))
}

#[derive(Deserialize, JsonSchema)]
pub struct LinkParams {
    pub document_id: String,
    /// claim | fact | opportunity | person
    pub target_kind: String,
    pub target_id: String,
    #[serde(default)]
    pub locator: Option<String>,
}

fn op_link(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: LinkParams = parse_params(p)?;
    require(scalar::<String, _>(ctx.conn, "SELECT id FROM documents WHERE id = ?1", [&p.document_id])?, "document", &p.document_id)?;
    let table = match p.target_kind.as_str() {
        "claim" => "claims",
        "fact" => "consumer_facts",
        "opportunity" => "opportunities",
        "person" => "people",
        _ => return Err(OcError::invalid("target_kind must be claim, fact, opportunity or person")),
    };
    require(scalar::<String, _>(ctx.conn, &format!("SELECT id FROM {table} WHERE id = ?1"), [&p.target_id])?, &p.target_kind, &p.target_id)?;
    let validated = ctx.actor.is_owner();
    exec(ctx.conn, "INSERT OR REPLACE INTO document_links(document_id, target_kind, target_id, locator, validated, created_at) VALUES (?1,?2,?3,?4,?5,?6)", (&p.document_id, &p.target_kind, &p.target_id, &p.locator, validated as i32, &ctx.now))?;
    Ok(json!({"document_id": p.document_id, "target_kind": p.target_kind, "target_id": p.target_id, "validated": validated, "note": if validated { "linked" } else { "linked as unvalidated; the owner validates before it counts as proof" }}))
}

fn op_redact(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: IdParam = parse_params(p)?;
    let body: Option<String> = scalar(ctx.conn, "SELECT body FROM documents_fts WHERE document_id = ?1", [&p.id])?;
    let Some(body) = body else { return Err(OcError::not_found("no indexed text for this document")) };
    let redacted = redact_text(&body);
    exec(ctx.conn, "DELETE FROM documents_fts WHERE document_id = ?1", [&p.id])?;
    exec(ctx.conn, "INSERT INTO documents_fts(document_id, body) VALUES (?1, ?2)", (&p.id, &redacted))?;
    Ok(json!({"document_id": p.id, "chars": redacted.chars().count()}))
}

#[derive(Deserialize, JsonSchema)]
pub struct ExportParams {
    pub id: String,
    /// Destination path for the decrypted original.
    pub to: String,
}

fn op_export(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: ExportParams = parse_params(p)?;
    let row = require(query_one(ctx.conn, "SELECT * FROM documents WHERE id = ?1", [&p.id])?, "document", &p.id)?;
    let dest = Path::new(&p.to);
    if dest.exists() {
        return Err(OcError::conflict("destination already exists"));
    }
    if !ctx.dry_run {
        let blob = std::fs::read(row["vault_path"].as_str().unwrap_or("")).map_err(|e| OcError::internal(e.to_string()))?;
        let bytes = ctx.svc.vault().decrypt(&blob).map_err(|e| OcError::internal(e.to_string()))?;
        std::fs::write(dest, bytes).map_err(|e| OcError::invalid(format!("write {}: {e}", dest.display())))?;
    }
    ctx.audit("documents.export", &p.id, json!({}))?;
    Ok(json!({"document_id": p.id, "exported_to": p.to}))
}

#[derive(Deserialize, JsonSchema)]
pub struct DeleteParams {
    pub id: String,
    pub confirm: bool,
}

fn op_delete(ctx: &mut Ctx<'_>, p: Value) -> OcResult<Value> {
    let p: DeleteParams = parse_params(p)?;
    if !p.confirm {
        return Err(OcError::invalid("pass confirm=true to permanently delete this document"));
    }
    let row = require(query_one(ctx.conn, "SELECT vault_path FROM documents WHERE id = ?1", [&p.id])?, "document", &p.id)?;
    exec(ctx.conn, "DELETE FROM documents_fts WHERE document_id = ?1", [&p.id])?;
    exec(ctx.conn, "DELETE FROM documents WHERE id = ?1", [&p.id])?;
    if !ctx.dry_run {
        let _ = std::fs::remove_file(row["vault_path"].as_str().unwrap_or(""));
    }
    ctx.audit("documents.delete", &p.id, json!({}))?;
    Ok(json!({"deleted": p.id}))
}

pub fn ops() -> Vec<OpSpec> {
    vec![
        op!("documents.import", "documents.import", Elevated, "Import a local file into the encrypted vault and queue extraction", ImportParams, op_import),
        op!("documents.list", "documents.metadata", Read, "List vault documents (metadata only)", PageParams, op_list, mcp = "list_document_metadata"),
        op!("documents.show", "documents.metadata", Read, "Document metadata, extraction state and links", IdParam, op_show, resource = "id"),
        op!("documents.extract", "documents.import", Write, "Re-run on-device extraction", IdParam, op_extract),
        op!("documents.suggestions", "documents.read", Read, "Deterministic fact suggestions with source spans", IdParam, op_suggestions, resource = "id"),
        op!("documents.search", "documents.read", Read, "Local full-text search over redacted extracted text", SearchParams, op_search),
        op!("documents.excerpt", "documents.read", Read, "Bounded, redacted excerpt of extracted text", ExcerptParams, op_excerpt, resource = "id", mcp = "read_document_excerpt"),
        op!("documents.link", "documents.metadata", Write, "Link a document to a claim, fact, opportunity or person", LinkParams, op_link),
        op!("documents.redact", "documents.import", Write, "Re-apply identifier redaction to the indexed text", IdParam, op_redact),
        op!("documents.export", "data.export", Elevated, "Decrypt a document to a path you choose", ExportParams, op_export),
        op!("documents.delete", "documents.delete", Elevated, "Permanently delete a document and its index", DeleteParams, op_delete),
    ]
}
