import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { ConfigEntry } from '../../core/api.types';
import { PrefsService } from '../../core/prefs.service';
import { ToastService } from '../../core/toast.service';
import { ConfirmService } from '../../shared/confirm.service';
import { PageHeaderComponent } from '../../shared/page-header.component';

interface Integrity { ok: boolean; quick_check: string; foreign_key_violations: number; audit_entries: number; audit_broken_at: number | null; documents_missing_files: number; recommendation: string }

const EDITABLE: Array<{ key: string; label: string; type: 'text' | 'bool' | 'number' | 'list' }> = [
  { key: 'network.contact', label: 'Network contact (User-Agent for SEC.gov)', type: 'text' },
  { key: 'network.offline', label: 'Offline mode (no network requests)', type: 'bool' },
  { key: 'vault.ocr', label: 'Use local Tesseract OCR when available', type: 'bool' },
  { key: 'vault.index_text', label: 'Index extracted text for local search', type: 'bool' },
  { key: 'alerts.lead_days', label: 'Reminder lead days (comma separated)', type: 'list' },
  { key: 'session.ttl_minutes', label: 'Owner session length (minutes)', type: 'number' },
  { key: 'gui.port', label: 'Localhost port (restart daemon to apply)', type: 'number' },
  { key: 'jobs.concurrency', label: 'Max background jobs', type: 'number' },
  { key: 'sources.refresh_hours', label: 'Refresh interval (hours)', type: 'number' },
];

@Component({
  selector: 'oc-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [PageHeaderComponent],
  template: `
    <oc-page-header title="Settings" subtitle="Daemon configuration, your passphrase, data safety, and how this GUI looks." />

    <div class="grid grid--2">
      <section class="card" aria-labelledby="cfg-h">
        <h2 id="cfg-h">Daemon configuration</h2>
        @for (f of fields; track f.key) {
          @if (config()[f.key]; as entry) {
            <div class="field">
              <label [for]="'cfg-' + f.key">{{ f.label }}</label>
              @if (f.type === 'bool') {
                <label class="field field--inline" style="margin:0"><input [id]="'cfg-' + f.key" type="checkbox" [checked]="!!draft()[f.key]" (change)="setDraft(f.key, $any($event.target).checked)" /> {{ draft()[f.key] ? 'on' : 'off' }}</label>
              } @else {
                <input [id]="'cfg-' + f.key" [type]="f.type === 'number' ? 'number' : 'text'" [value]="draft()[f.key] ?? ''" (input)="setDraft(f.key, $any($event.target).value)" />
              }
              <span class="hint">{{ entry.description }} Default: <code>{{ show(entry.default) }}</code></span>
              <div><button class="btn btn--sm" type="button" (click)="save(f.key, f.type)" [disabled]="busy()">Save</button></div>
            </div>
          }
        }
      </section>

      <div class="stack">
        <section class="card" aria-labelledby="look-h">
          <h2 id="look-h">Appearance</h2>
          <div class="field"><label for="theme">Theme</label>
            <select id="theme" [value]="prefs.theme()" (change)="prefs.theme.set($any($event.target).value)"><option value="system">Follow system</option><option value="dark">Dark</option><option value="light">Light</option></select></div>
          <div class="field"><label for="font">Font size ({{ (prefs.fontScale() * 100).toFixed(0) }}%)</label>
            <input id="font" type="range" min="0.8" max="1.6" step="0.1" [value]="prefs.fontScale()" (input)="prefs.fontScale.set(+$any($event.target).value)" /></div>
          <div class="field"><label for="motion">Motion</label>
            <select id="motion" [value]="prefs.motion()" (change)="prefs.motion.set($any($event.target).value)"><option value="system">Follow system</option><option value="reduced">Reduced</option><option value="full">Full</option></select></div>
          <p class="hint">Stored only in this browser's localStorage. Nothing else is stored client-side.</p>
        </section>

        <section class="card" aria-labelledby="pp-h">
          <h2 id="pp-h">Change passphrase</h2>
          <form (submit)="changePassphrase($event)">
            <div class="field"><label for="cur">Current passphrase</label><input id="cur" type="password" autocomplete="current-password" [value]="cur()" (input)="cur.set($any($event.target).value)" required /></div>
            <div class="field"><label for="new1">New passphrase</label><input id="new1" type="password" autocomplete="new-password" [value]="new1()" (input)="new1.set($any($event.target).value)" required minlength="8" /></div>
            <div class="field"><label for="new2">Repeat new passphrase</label><input id="new2" type="password" autocomplete="new-password" [value]="new2()" (input)="new2.set($any($event.target).value)" required /></div>
            <button class="btn btn--primary" type="submit" [disabled]="busy() || !cur() || new1().length < 8 || new1() !== new2()">Change passphrase</button>
          </form>
        </section>
      </div>

      <section class="card" aria-labelledby="data-h">
        <h2 id="data-h">Data safety</h2>
        <form class="row" (submit)="backup($event)">
          <div class="field" style="flex:1; margin:0"><label for="bk">Backup to absolute path</label><input id="bk" type="text" [value]="backupPath()" (input)="backupPath.set($any($event.target).value)" placeholder="C:\\Users\\you\\openclaim-backup.db" /></div>
          <button class="btn" type="submit" [disabled]="!backupPath().trim() || busy()" style="align-self:flex-end">Backup</button>
        </form>
        <form class="row" (submit)="exportData($event)" style="margin-top:0.5rem">
          <div class="field" style="flex:1; margin:0"><label for="ex">Export your records (JSON) to absolute path</label><input id="ex" type="text" [value]="exportPath()" (input)="exportPath.set($any($event.target).value)" placeholder="C:\\Users\\you\\openclaim-export.json" /></div>
          <button class="btn" type="submit" [disabled]="busy()" style="align-self:flex-end">Export</button>
        </form>
        <div style="margin-top:0.75rem"><button class="btn" type="button" (click)="integrity()" [disabled]="busy()">Check integrity</button></div>
        @if (integrityResult(); as i) {
          <dl class="dl" style="margin-top:0.5rem">
            <dt>Status</dt><dd>{{ i.ok ? 'OK' : 'Problems found' }} ({{ i.quick_check }})</dd>
            <dt>Audit entries</dt><dd>{{ i.audit_entries }}{{ i.audit_broken_at !== null ? ' — chain broken at ' + i.audit_broken_at : ' — chain intact' }}</dd>
            <dt>Missing files</dt><dd>{{ i.documents_missing_files }}</dd>
            <dt>Recommendation</dt><dd>{{ i.recommendation }}</dd>
          </dl>
        }
        <hr style="border-color: var(--border); margin: 1rem 0" />
        <h3>Wipe local data</h3>
        <p class="small muted">Deletes people, facts, claims, documents and audit from this computer. The public catalog can be kept.</p>
        <label class="field field--inline"><input type="checkbox" [checked]="wipeCatalog()" (change)="wipeCatalog.set($any($event.target).checked)" /> Also wipe the opportunity catalog</label>
        <button class="btn btn--danger" type="button" (click)="wipe()" [disabled]="busy()">Wipe…</button>
      </section>

      <section class="card" aria-labelledby="about-h">
        <h2 id="about-h">About</h2>
        <p><strong>No AI inside.</strong> OpenClaim ships no language model, inference runtime, embeddings or AI API client. Search is SQLite full-text search with deterministic ranking; eligibility matching is a deterministic rules engine that cites its reasons. External agents may call the CLI or MCP adapter, but every proposal they make stays untrusted until you confirm it.</p>
        <p><strong>Free and local.</strong> Every source is free and official. Data never leaves this computer unless you export it. OpenClaim never submits a claim for you and takes no commission.</p>
        <p class="small muted">Daemon: {{ status()?.version || '…' }} · schema {{ status()?.schema_version || '…' }} · data dir <code>{{ status()?.data_dir || '…' }}</code></p>
        <p class="small muted">{{ status()?.ai_integration }}</p>
      </section>
    </div>
  `,
})
export class SettingsPage implements OnInit {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly auth = inject(AuthService);
  readonly prefs = inject(PrefsService);

  readonly fields = EDITABLE;
  readonly config = signal<Record<string, ConfigEntry>>({});
  readonly draft = signal<Record<string, unknown>>({});
  readonly status = signal<{ version: string; schema_version: string; data_dir: string; ai_integration: string } | null>(null);
  readonly integrityResult = signal<Integrity | null>(null);
  readonly busy = signal(false);
  readonly cur = signal(''); readonly new1 = signal(''); readonly new2 = signal('');
  readonly backupPath = signal(''); readonly exportPath = signal('');
  readonly wipeCatalog = signal(false);

  async ngOnInit(): Promise<void> {
    try {
      const [cfg, st] = await Promise.all([this.api.op<Record<string, ConfigEntry>>('config.show', {}), this.api.op<{ version: string; schema_version: string; data_dir: string; ai_integration: string }>('daemon.status', {})]);
      this.config.set(cfg);
      const d: Record<string, unknown> = {};
      for (const f of EDITABLE) { const v = cfg[f.key]?.value; d[f.key] = f.type === 'list' && Array.isArray(v) ? v.join(', ') : v ?? ''; }
      this.draft.set(d);
      this.status.set(st);
    } catch (e) { this.toast.error(e); }
  }

  show(v: unknown): string { return v === null || v === undefined ? 'none' : Array.isArray(v) ? v.join(', ') : String(v); }
  setDraft(key: string, v: unknown): void { this.draft.update((d) => ({ ...d, [key]: v })); }

  async save(key: string, type: 'text' | 'bool' | 'number' | 'list'): Promise<void> {
    const raw = this.draft()[key];
    let value: unknown = raw;
    if (type === 'number') value = Number(raw);
    if (type === 'list') value = String(raw).split(',').map((s) => s.trim()).filter(Boolean).map(Number);
    if (type === 'text' && raw === '') value = null;
    this.busy.set(true);
    try { await this.api.op('config.set', { key, value }); this.toast.ok(`${key} saved.`); } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async changePassphrase(ev: Event): Promise<void> {
    ev.preventDefault();
    this.busy.set(true);
    try {
      await this.api.op('owner.set_passphrase', { current: this.cur(), new: this.new1() });
      this.toast.ok('Passphrase changed. Unlock again with the new passphrase.');
      this.cur.set(''); this.new1.set(''); this.new2.set('');
      await this.auth.logout();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async backup(ev: Event): Promise<void> {
    ev.preventDefault();
    this.busy.set(true);
    try { await this.api.op('data.backup', { path: this.backupPath().trim() }); this.toast.ok('Backup written.'); } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async exportData(ev: Event): Promise<void> {
    ev.preventDefault();
    this.busy.set(true);
    try {
      const p = this.exportPath().trim();
      const r = await this.api.op<unknown>('data.export', { format: 'json', path: p || null });
      if (!p) {
        const blob = new Blob([JSON.stringify(r, null, 2)], { type: 'application/json' });
        const a = document.createElement('a'); a.href = URL.createObjectURL(blob); a.download = 'openclaim-export.json'; a.click(); URL.revokeObjectURL(a.href);
      }
      this.toast.ok('Export complete.');
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async integrity(): Promise<void> {
    this.busy.set(true);
    try { this.integrityResult.set(await this.api.op<Integrity>('data.integrity', {})); } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async wipe(): Promise<void> {
    const ok = await this.confirm.ask({ title: 'Wipe local data?', message: 'This permanently deletes your local records. Make a backup first. This cannot be undone.', confirmLabel: 'Wipe everything', danger: true, typeToConfirm: 'WIPE' });
    if (!ok) return;
    this.busy.set(true);
    try { await this.api.op('data.wipe', { confirm: 'WIPE', include_catalog: this.wipeCatalog() }); this.toast.ok('Local data wiped.'); await this.auth.logout(); } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }
}
