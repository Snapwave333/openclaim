import { Routes } from '@angular/router';
import { authGuard } from './core/auth.guard';

export const routes: Routes = [
  { path: 'login', loadComponent: () => import('./pages/login/login.page').then((m) => m.LoginPage), title: 'Sign in · OpenClaim' },
  {
    path: '',
    canActivate: [authGuard],
    children: [
      { path: '', pathMatch: 'full', loadComponent: () => import('./pages/overview/overview.page').then((m) => m.OverviewPage), title: 'Overview · OpenClaim' },
      { path: 'discover', loadComponent: () => import('./pages/discover/discover.page').then((m) => m.DiscoverPage), title: 'Discover · OpenClaim' },
      { path: 'discover/:id', loadComponent: () => import('./pages/discover/opportunity-detail.page').then((m) => m.OpportunityDetailPage), title: 'Opportunity · OpenClaim' },
      { path: 'matches', loadComponent: () => import('./pages/matches/matches.page').then((m) => m.MatchesPage), title: 'My Matches · OpenClaim' },
      { path: 'claims', loadComponent: () => import('./pages/claims/claims.page').then((m) => m.ClaimsPage), title: 'Claims · OpenClaim' },
      { path: 'claims/:id', loadComponent: () => import('./pages/claims/claim-detail.page').then((m) => m.ClaimDetailPage), title: 'Claim · OpenClaim' },
      { path: 'vault', loadComponent: () => import('./pages/vault/vault.page').then((m) => m.VaultPage), title: 'Evidence Vault · OpenClaim' },
      { path: 'sources', loadComponent: () => import('./pages/sources/sources.page').then((m) => m.SourcesPage), title: 'Sources · OpenClaim' },
      { path: 'agents', loadComponent: () => import('./pages/agents/agents.page').then((m) => m.AgentsPage), title: 'Agent Activity · OpenClaim' },
      { path: 'approvals', loadComponent: () => import('./pages/approvals/approvals.page').then((m) => m.ApprovalsPage), title: 'Approvals · OpenClaim' },
      { path: 'settings', loadComponent: () => import('./pages/settings/settings.page').then((m) => m.SettingsPage), title: 'Settings · OpenClaim' },
    ],
  },
  { path: '**', redirectTo: '' },
];
