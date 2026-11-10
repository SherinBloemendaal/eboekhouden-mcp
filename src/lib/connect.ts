import { invoke } from '@tauri-apps/api/core'
import { inApp } from './server'

export type Client = 'claude' | 'claudeCode' | 'cursor' | 'codex'
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
  legacy: boolean
  entry: Record<string, unknown>
  /** Codex: the TOML table that will be written. */
  toml: string | null
  backup: string
}

export const CLIENT_NAMES: Record<Client, string> = { claude: 'Claude', claudeCode: 'Claude Code', cursor: 'Cursor', codex: 'Codex' }

/** The server's name in each client's config (connect.rs, name()). Claude Code and Codex only take letters, digits, - and _. */
export const SERVER_NAMES: Record<Client, string> = { claude: 'e-Boekhouden MCP', cursor: 'e-Boekhouden MCP', claudeCode: 'e-boekhouden', codex: 'e-boekhouden' }

/** Clients that start the bridge, so their config holds no token. */
export const usesBridge = (client: Client): boolean => client === 'claude' || client === 'codex'

export const clientsStatus = () => (inApp() ? invoke<ClientStatus[]>('clients_status') : Promise.resolve<ClientStatus[]>([]))
export const preview = (client: Client) => invoke<Preview>('connect_preview', { client })
export const link = (client: Client) => invoke('connect_link', { client })
export const unlink = (client: Client) => invoke('connect_unlink', { client })
export const revealToken = () => invoke<string>('token_reveal')
export const renewToken = () => invoke<string>('token_renew')
export const otherClientSettings = () => invoke<string>('other_client_settings')
