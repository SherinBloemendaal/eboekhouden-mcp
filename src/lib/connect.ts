import { invoke } from '@tauri-apps/api/core'
import { inApp } from './server'

export type Client = 'claude' | 'claudeCode' | 'cursor'
export type LinkState = 'none' | 'linked' | 'stale' | 'broken'

export interface ClientStatus {
  client: Client
  path: string
  state: LinkState
  error: string | null
}

export interface Preview {
  client: Client
  path: string
  exists: boolean
  otherServers: string[]
  replaces: boolean
  entry: Record<string, unknown>
  backup: string
}

export const CLIENT_NAMES: Record<Client, string> = { claude: 'Claude', claudeCode: 'Claude Code', cursor: 'Cursor' }

export const clientsStatus = () => (inApp() ? invoke<ClientStatus[]>('clients_status') : Promise.resolve<ClientStatus[]>([]))
export const preview = (client: Client) => invoke<Preview>('connect_preview', { client })
export const link = (client: Client) => invoke('connect_link', { client })
export const unlink = (client: Client) => invoke('connect_unlink', { client })
export const revealToken = () => invoke<string>('token_reveal')
export const renewToken = () => invoke<string>('token_renew')
export const otherClientSettings = () => invoke<string>('other_client_settings')
