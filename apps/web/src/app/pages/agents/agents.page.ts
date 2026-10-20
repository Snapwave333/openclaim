import { ChangeDetectionStrategy, Component, OnDestroy, OnInit, inject, signal } from '@angular/core';
import { Subscription } from 'rxjs';
import { ApiService } from '../../core/api.service';
import { Agent, Job, Paged, Scope } from '../../core/api.types';
import { EventsService } from '../../core/events.service';
import { date, humanize, relative } from '../../core/format';
import { ToastService } from '../../core/toast.service';
import { BadgeComponent } from '../../shared/badge.component';
import { ConfirmService } from '../../shared/confirm.service';
import { PageHeaderComponent } from '../../shared/page-header.component';
import { TermComponent } from '../../shared/term.component';

interface Sessions { agents: Array<Record<string, unknown>>; owner_sessions: Array<{ transport: string; expires_at: string }> }
interface AuditRow { seq?: number; id?: string; actor?: string; action?: string; timestamp?: string; redacted_subject?: string; details?: unknown; [k: string]: unknown }

@Component({
  selector: 'oc-agents',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BadgeComponent, PageHeaderComponent, TermComponent],
  template: `
    <oc-page-header title="Agent Activity" subtitle="External CLI and MCP agents that may use this daemon. Research is read-only by default; anything else needs your approval.">
      <button class="btn" type="button" (click)="registering.set(!registering())">Register agent</button>
    </oc-page-header>

    @if (registering()) {
      <form class="card" (submit)="register($event)" style="margin-bottom:1rem">
        <h2>Register an agent</h2>
        <div class="row">
          <div class="field" style="margin:0"><label for="ag-name">Name</label><input id="ag-name" type="text" [value]="regName()" (input)="regName.set($any($event.target).value)" required /></div>
          <div class="field" style="margin:0"><label for="ag-kind">Kind</label><select id="ag-kind" [value]="regKind()" (change)="regKind.set($any($event.target).value)"><option value="agent">agent (CLI/MCP)</option><option value="extension">browser extension</option></select></div>
          @if (regKind() === 'extension') { <div class="field" style="margin:0"><label for="ag-origin">Allowed origin</label><input id="ag-origin" type="text" [value]="regOrigin()" (input)="regOrigin.set($any($event.target).value)" placeholder="chrome-extension://…" /></div> }
          <div class="field" style="margin:0"><label for="ag-exp">Expires in days</label><input id="ag-exp" type="number" min="1" [value]="regDays()" (input)="regDays.set($any($event.target).value)" style="width:6rem" /></div>
        </div>
        <fieldset><legend><oc-term key="scope">Scopes</oc-term></legend>
          <div class="grid grid--3">
            @for (s of scopes(); track s.scope) {
              <label class="field field--inline" style="margin:0"><input type="checkbox" [checked]="regScopes().includes(s.scope)" (change)="toggleScope(regScopes, s.scope, $any($event.target).checked)" [disabled]="s.owner_only" /> <span><code>{{ s.scope }}</code><br /><span class="small muted">{{ s.description }}{{ s.owner_only ? ' (owner only)' : '' }}</span></span></label>
            }
          </div>
        </fieldset>
        <div class="row"><button class="btn btn--primary" type="submit" [disabled]="!regName().trim() || regScopes().length === 0 || busy()">Register</button><button class="btn btn--ghost" type="button" (click)="registering.set(false)">Cancel</button></div>
      </form>
    }

    @if (token(); as t) {
      <div class="card notice notice--warn" role="alert" style="margin-bottom:1rem">
        <h2>Agent token — shown once</h2>
        <p>Copy it now and give it to the agent as <code>OPENCLAIM_AGENT_TOKEN</code>. OpenClaim does not store the plain token; if it is lost, revoke the agent and register again.</p>
        <p class="row"><code class="card" style="padding:0.4rem 0.6rem">{{ t }}</code><button class="btn btn--sm" type="button" (click)="copy(t)">Copy</button><button class="btn btn--sm btn--ghost" type="button" (click)="token.set(null)">I saved it</button></p>
      </div>
    }

    <section class="card" aria-labelledby="ag-h">
      <h2 id="ag-h">Agents</h2>
      <div class="table-wrap">
        <table>
          <thead><tr><th scope="col">Name</th><th scope="col">Kind</th><th scope="col">Scopes</th><th scope="col">Expires</th><th scope="col">State</th><th scope="col">Actions</th></tr></thead>
          <tbody>
            @for (a of agents(); track a.id) {
              <tr [attr.data-agent-id]="a.id">
                <td>{{ a.display_name }}<div class="small faint mono">{{ a.id }}</div></td>
                <td>{{ a.kind }}</td>
                <td class="small">@for (s of a.scopes; track s) { <code>{{ s }}</code> }</td>
                <td class="small">{{ a.expires_at ? date(a.expires_at) : 'never' }}</td>
                <td><oc-badge [value]="a.revoked_at ? 'revoked' : 'active'" /></td>
                <td class="row">
                  <button class="btn btn--sm" type="button" (click)="openScopes(a)" [disabled]="!!a.revoked_at">Scopes</button>
                  <button class="btn btn--sm" type="button" (click)="openAudit(a)">Audit</button>
                  @if (!a.revoked_at) { <button class="btn btn--sm btn--danger" type="button" (click)="revoke(a)" [disabled]="busy()">Revoke (kill switch)</button> }
                </td>
              </tr>
            } @empty { <tr><td colspan="6" class="muted">No agents registered. Owners act directly; agents need a token.</td></tr> }
          </tbody>
        </table>
      </div>
    </section>

    @if (scopesFor(); as a) {
      <section class="section card" aria-labelledby="sc-h">
        <div class="row row--between"><h2 id="sc-h">Scopes for {{ a.display_name }}</h2><button class="btn btn--sm btn--ghost" type="button" (click)="scopesFor.set(null)">Close</button></div>
        <div class="grid grid--3">
          @for (s of scopes(); track s.scope) {
            <label class="field field--inline" style="margin:0"><input type="checkbox" [checked]="editScopes().includes(s.scope)" (change)="toggleScope(editScopes, s.scope, $any($event.target).checked)" [disabled]="s.owner_only" /> <span><code>{{ s.scope }}</code><br /><span class="small muted">{{ s.description }}</span></span></label>
          }
        </div>
        <button class="btn btn--primary" type="button" (click)="saveScopes(a)" [disabled]="busy()" style="margin-top:0.75rem">Save scopes</button>
      </section>
    }

    @if (auditFor(); as a) {
      <section class="section card" aria-labelledby="au-h">
        <div class="row row--between"><h2 id="au-h">Audit for {{ a.display_name }}</h2><button class="btn btn--sm btn--ghost" type="button" (click)="auditFor.set(null)">Close</button></div>
        <ul class="plain">
          @for (r of audit(); track $index) {
            <li><strong>{{ r.action || r['command'] || r['kind'] }}</strong> <span class="small muted">{{ date(r.timestamp || rowAt(r)) }}</span>@if (r.redacted_subject && r.redacted_subject !== '-') { <span class="small"> · {{ r.redacted_subject }}</span> }<details class="small"><summary>Details</summary><pre class="mono">{{ pretty(r) }}</pre></details></li>
          } @empty { <li class="muted">No audit entries.</li> }
        </ul>
      </section>
    }

    <section class="section card" aria-labelledby="se-h">
      <h2 id="se-h">Sessions</h2>
      <p class="small muted">Owner sessions: @for (s of sessions()?.owner_sessions ?? []; track $index) { <span class="badge">{{ s.transport }} · expires {{ relative(s.expires_at) }}</span> } @if (!sessions()?.owner_sessions?.length) { none }</p>
      <p class="small muted">Agent sessions: {{ sessions()?.agents?.length ?? 0 }}</p>
      @for (s of sessions()?.agents ?? []; track $index) { <pre class="mono small">{{ pretty(s) }}</pre> }
    </section>

    <section class="section card" aria-labelledby="jobs-h">
      <div class="row row--between"><h2 id="jobs-h">Jobs</h2><span class="small muted">live via events: {{ events.connected() ? 'connected' : 'reconnecting' }}</span></div>
      <p class="hint">Jobs are durable: closing this GUI never cancels them.</p>
      <div class="table-wrap">
        <table>
          <thead><tr><th scope="col">Kind</th><th scope="col">State</th><th scope="col">Progress</th><th scope="col">Attempts</th><th scope="col">Owner</th><th scope="col">Updated</th><th scope="col">Actions</th></tr></thead>
          <tbody>
            @for (j of jobs(); track j.id) {
              <tr [attr.data-job-id]="j.id">
                <td>{{ humanize(j.kind) }}<div class="small faint mono">{{ j.id.slice(0, 8) }}</div></td>
                <td><oc-badge [value]="j.state" />@if (j.error_message) { <div class="small muted">{{ j.error_code }}: {{ j.error_message }}</div> }</td>
                <td style="min-width: 8rem"><div class="progress" role="progressbar" [attr.aria-valuenow]="j.progress" aria-valuemin="0" aria-valuemax="100" [attr.aria-label]="'progress ' + j.progress + '%'"><span [style.width.%]="j.progress"></span></div><span class="small">{{ j.progress }}%</span></td>
                <td>{{ j.attempts }}/{{ j.max_attempts }}</td>
                <td class="small">{{ j.owner_actor_id }}</td>
                <td class="small muted">{{ relative(j.updated_at) }}</td>
                <td class="row">
                  @if (j.state === 'running' || j.state === 'queued' || j.state === 'pending') { <button class="btn btn--sm" type="button" (click)="cancel(j.id)">Cancel</button> }
                  @if (j.state === 'failed' || j.state === 'cancelled') { <button class="btn btn--sm" type="button" (click)="retry(j.id)">Retry</button> }
                </td>
              </tr>
            } @empty { <tr><td colspan="7" class="muted">No jobs.</td></tr> }
          </tbody>
        </table>
      </div>
    </section>
  `,
})
export class AgentsPage implements OnInit, OnDestroy {
  private readonly api = inject(ApiService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  readonly events = inject(EventsService);
  private sub?: Subscription;

  readonly agents = signal<Agent[]>([]);
  readonly scopes = signal<Scope[]>([]);
  readonly sessions = signal<Sessions | null>(null);
  readonly jobs = signal<Job[]>([]);
  readonly busy = signal(false);
  readonly registering = signal(false);
  readonly regName = signal('');
  readonly regKind = signal('agent');
  readonly regOrigin = signal('');
  readonly regDays = signal('30');
  readonly regScopes = signal<string[]>(['discover.read', 'opportunities.read', 'sources.read']);
  readonly token = signal<string | null>(null);
  readonly scopesFor = signal<Agent | null>(null);
  readonly editScopes = signal<string[]>([]);
  readonly auditFor = signal<Agent | null>(null);
  readonly audit = signal<AuditRow[]>([]);
  readonly humanize = humanize;
  readonly date = date;
  readonly relative = relative;

  async ngOnInit(): Promise<void> {
    await this.reload();
    this.sub = this.events.events$.subscribe((e) => {
      if (e.topic === 'job.progress' || e.topic === 'job.completed') {
        const p = e.payload as Partial<Job> & { job_id?: string };
        const id = p.id ?? p.job_id;
        if (id && this.jobs().some((j) => j.id === id)) {
          this.jobs.update((l) => l.map((j) => (j.id === id ? { ...j, ...(p.progress !== undefined ? { progress: p.progress } : {}), ...(p.state ? { state: p.state } : {}), updated_at: e.at } : j)));
        } else {
          void this.loadJobs();
        }
      }
    });
  }
  ngOnDestroy(): void { this.sub?.unsubscribe(); }

  async reload(): Promise<void> {
    try {
      const [agents, scopes, sessions] = await Promise.all([this.api.op<Paged<Agent>>('agents.list', {}), this.api.op<{ scopes: Scope[] }>('agents.scopes', {}), this.api.op<Sessions>('agents.sessions', {})]);
      this.agents.set(agents.items ?? []);
      this.scopes.set(scopes.scopes ?? []);
      this.sessions.set(sessions);
      await this.loadJobs();
    } catch (e) { this.toast.error(e); }
  }

  async loadJobs(): Promise<void> {
    try { this.jobs.set((await this.api.op<Paged<Job>>('jobs.list', { limit: 50 })).items ?? []); } catch (e) { this.toast.error(e); }
  }

  toggleScope(sig: { update: (fn: (l: string[]) => string[]) => void }, scope: string, on: boolean): void {
    sig.update((l) => (on ? [...new Set([...l, scope])] : l.filter((x) => x !== scope)));
  }

  async register(ev: Event): Promise<void> {
    ev.preventDefault();
    this.busy.set(true);
    try {
      const r = await this.api.op<{ token: string }>('agents.register', { name: this.regName().trim(), scopes: this.regScopes(), kind: this.regKind(), allowed_origin: this.regKind() === 'extension' ? this.regOrigin().trim() || null : null, expires_in_days: this.regDays() ? Number(this.regDays()) : null });
      this.token.set(r.token);
      this.registering.set(false);
      this.regName.set('');
      await this.reload();
    } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async copy(t: string): Promise<void> {
    try { await navigator.clipboard.writeText(t); this.toast.ok('Token copied.'); } catch { this.toast.show('Clipboard unavailable; select and copy the token manually.'); }
  }

  openScopes(a: Agent): void { this.scopesFor.set(a); this.editScopes.set([...a.scopes]); }
  async saveScopes(a: Agent): Promise<void> {
    const add = this.editScopes().filter((s) => !a.scopes.includes(s));
    const remove = a.scopes.filter((s) => !this.editScopes().includes(s));
    this.busy.set(true);
    try { await this.api.op('agents.authorize', { id: a.id, add_scopes: add, remove_scopes: remove }); this.toast.ok('Scopes updated.'); this.scopesFor.set(null); await this.reload(); } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async openAudit(a: Agent): Promise<void> {
    this.auditFor.set(a);
    try {
      const r = await this.api.op<Record<string, unknown>>('agents.audit', { id: a.id });
      const rows = (r['items'] ?? r['entries'] ?? r['commands'] ?? []) as AuditRow[];
      const disclosures = (r['disclosures'] as AuditRow[] | undefined) ?? [];
      this.audit.set([...rows, ...disclosures.map((d) => ({ ...d, action: `disclosure: ${String(d['kind'] ?? d['field'] ?? '')}` }))]);
    } catch (e) { this.toast.error(e); }
  }

  async revoke(a: Agent): Promise<void> {
    if (!(await this.confirm.ask({ title: `Revoke ${a.display_name}?`, message: 'The token stops working immediately, running sessions end and pending approvals from this agent are void. This cannot be undone.', confirmLabel: 'Revoke now', danger: true }))) return;
    this.busy.set(true);
    try { await this.api.op('agents.revoke', { id: a.id }); this.toast.ok('Agent revoked.'); await this.reload(); } catch (e) { this.toast.error(e); } finally { this.busy.set(false); }
  }

  async cancel(id: string): Promise<void> { try { await this.api.op('jobs.cancel', { job_id: id }); await this.loadJobs(); } catch (e) { this.toast.error(e); } }
  async retry(id: string): Promise<void> { try { await this.api.op('jobs.retry', { job_id: id }); await this.loadJobs(); } catch (e) { this.toast.error(e); } }
  rowAt(r: AuditRow): string | null { const v = r['at']; return typeof v === 'string' ? v : null; }
  pretty(v: unknown): string { return JSON.stringify(v, null, 2); }
}
