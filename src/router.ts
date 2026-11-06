import { createRouter, createWebHashHistory } from 'vue-router'
import AppShell from './components/AppShell.vue'
import AccountsView from './views/AccountsView.vue'
import ConnectView from './views/ConnectView.vue'
import DashboardView from './views/DashboardView.vue'
import PanelView from './views/PanelView.vue'
import RequestsView from './views/RequestsView.vue'
import SettingsView from './views/SettingsView.vue'

// Hash history: the app is served from files, and the Rust side opens a page by setting the hash.
export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/panel', component: PanelView },
    {
      path: '/',
      component: AppShell,
      redirect: '/dashboard',
      children: [
        { path: 'dashboard', component: DashboardView, meta: { title: 'Dashboard' } },
        { path: 'accounts', component: AccountsView, meta: { title: 'Accounts' } },
        { path: 'verzoeken', component: RequestsView, meta: { title: 'Verzoeken' } },
        { path: 'koppelen', component: ConnectView, meta: { title: 'Koppelen' } },
        { path: 'instellingen', component: SettingsView, meta: { title: 'Instellingen' } },
      ],
    },
  ],
})
