import {defineStore} from 'pinia'
import {AppSettings} from '@/models/settings';
import {getSettings, pathExists, putSettings} from '@/api/client';

/**
 * The application settings, over HTTP.
 *
 * Everything here used to go through Tauri's `invoke`. Three of the four
 * commands had a route waiting for them; the fourth, `maximize_window`, is a
 * desktop-window operation that a page served into a browser tab cannot
 * perform and has no analogue for, so it is gone rather than ported.
 *
 * Errors are deliberately not caught. Every failure arrives as the `ApiError`
 * `request` throws, with the server's application `code` intact, and the views
 * decide what to show. A store that swallowed them would leave the UI showing
 * settings that were never persisted.
 *
 * None of the three routes this store uses can answer `code: 2` ("no Factorio
 * instance is running") -- that condition belongs to rcon, instance/stop and
 * scripts/execute. There is consequently nothing here to branch on, and in
 * particular nothing that should ever branch on an HTTP status: `code: 2` is a
 * `400` from two routes and a `503` from a third.
 */
export const useAppStore = defineStore('app', {
  state: () => ({
    settings: null as AppSettings | null
  }),
  getters: {
    getSettings(): AppSettings | null {
      return this.settings
    },
    getWorkspacePath(): string | null {
      return this.settings ? this.settings.factorio.workspace_path : null
    },
    getRecreateLevel(): boolean | undefined {
      return this.settings ? this.settings.factorio.recreate : undefined
    },
    getEnableAutostart(): boolean | null {
      return this.settings ? this.settings.gui.enable_autostart : null
    },
    getRestapiPort(): number | null {
      // `restapi` is non-optional in `AppSettings`, so the `?.` this line used
      // to carry could never short-circuit.
      return this.settings ? this.settings.restapi.port : null
    },
    getFactorioArchivePath(): string | null {
      return this.settings ? this.settings.factorio.factorio_archive_path : null
    },
    getClientCount(): number | null {
      return this.settings ? this.settings.factorio.client_count : null
    },
    getMapExchangeString(): string | null {
      return this.settings ? this.settings.factorio.map_exchange_string : null
    },
    getSeed(): string | null {
      return this.settings ? this.settings.factorio.seed : null
    }
  },
  actions: {
    /**
     * Whether a path exists on the *server's* filesystem.
     *
     * A browser cannot see that filesystem, which is the same reason the
     * native file pickers cannot survive the move: the paths this app
     * configures are the server's, not the viewer's.
     */
    async fileExists(path: string): Promise<boolean> {
      const response = await pathExists(path)
      return response.exists
    },
    /**
     * The assignment happens after the await, so a failed load rejects with
     * the settings that were already there untouched rather than blanking the
     * UI over a transient network failure.
     */
    async loadSettings(): Promise<AppSettings> {
      const settings = await getSettings()
      this.settings = settings
      return settings
    },
    /**
     * PUT answers with the settings as persisted. Adopting that answer instead
     * of keeping the local object means any normalisation the server applies
     * is visible immediately, rather than at the next reload.
     *
     * The null check is what narrows `settings` for the call; every caller
     * below has already established it, so it is a type guard and not a branch
     * any test can reach.
     */
    async _updateSettings() {
      if (this.settings === null) {
        return
      }
      this.settings = await putSettings(this.settings)
    },
    async updateWorkspacePath(workspacePath: string) {
      if (this.settings !== null) {
        this.settings.factorio.workspace_path = workspacePath
        await this._updateSettings()
      }
    },
    async updateFactorioArchivePath(factorioArchivePath: string) {
      if (this.settings !== null) {
        this.settings.factorio.factorio_archive_path = factorioArchivePath
        await this._updateSettings()
      }
    },
    async updateRecreateLevel(recreateLevel: boolean) {
      if (this.settings !== null) {
        this.settings.factorio.recreate = recreateLevel
        await this._updateSettings()
      }
    },
    async updateEnableAutostart(enableAutostart: boolean) {
      if (this.settings !== null) {
        this.settings.gui.enable_autostart = enableAutostart
        await this._updateSettings()
      }
    },
    /**
     * Persisted only.
     *
     * This used to stop and restart an in-process REST API through
     * `restapiStore`, which is incoherent now: the server whose port this
     * names is the one serving this page. The bind port is chosen when
     * `factorio-bot serve` starts, so a change here takes effect on the next
     * start.
     */
    async updateRestapiPort(restapiPort: number) {
      if (this.settings !== null) {
        this.settings.restapi.port = restapiPort
        await this._updateSettings()
      }
    },
    async updateClientCount(clientCount: number) {
      if (this.settings !== null) {
        this.settings.factorio.client_count = clientCount
        await this._updateSettings()
      }
    },
    async updateMapExchangeString(mapExchangeString: string) {
      if (this.settings !== null) {
        this.settings.factorio.map_exchange_string = mapExchangeString
        await this._updateSettings()
      }
    },
    async updateSeed(seed: string) {
      if (this.settings !== null) {
        this.settings.factorio.seed = seed
        await this._updateSettings()
      }
    }
  }
})
