import { defineStore } from 'pinia'
import {AppSettings} from '@/models/settings';
import {invoke} from '@tauri-apps/api/core';
import {useRestApiStore} from '@/store/restapiStore';

export const useAppStore = defineStore({
  id: 'app',
  state: () => ({
    settings: null as AppSettings | null
  }),
  getters: {
    getSettings(): AppSettings | null {
      return this.settings
    },
    getWorkspacePath(): string | null {
      if (this.settings) {
        return this.settings.factorio.workspace_path
      } else {
        return null
      }
    },
    getRecreateLevel(): boolean | undefined {
      if (this.settings) {
        return this.settings.factorio.recreate
      }
    },
    getEnableRestapi(): boolean | null {
      if (this.settings) {
        return this.settings.gui.enable_restapi
      } else {
        return null
      }
    },
    getEnableAutostart(): boolean | null {
      if (this.settings) {
        return this.settings.gui.enable_autostart
      } else {
        return null
      }
    },
    getRestapiPort(): number | null {
      if (this.settings) {
        return this.settings.restapi?.port
      } else {
        return null
      }
    },
    getFactorioArchivePath(): string | null {
      if (this.settings) {
        return this.settings.factorio.factorio_archive_path
      } else {
        return null
      }
    },
    getClientCount(): number | null {
      if (this.settings) {
        return this.settings.factorio.client_count
      } else {
        return null
      }
    },
    getMapExchangeString(): string | null {
      if (this.settings) {
        return this.settings.factorio.map_exchange_string
      } else {
        return null
      }
    },
    getSeed(): string | null {
      if (this.settings) {
        return this.settings.factorio.seed
      } else {
        return null
      }
    }
  },
  actions: {
    async fileExists(path: string) {
      return await invoke('file_exists', {path})
    },
    async loadSettings() {
      this.settings = await invoke('load_settings')
      return this.settings
    },
    async maximizeWindow() {
      this.settings = await invoke('maximize_window')
    },
    async _updateSettings() {
      await invoke('update_settings', {settings: this.settings})
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
    async updateEnableRestApi(enableRestapi: boolean) {
      if (this.settings !== null) {
        const restApiStore = useRestApiStore()
        if (this.settings.gui.enable_restapi && !enableRestapi) {
          await restApiStore.stopRestApi()
        } else if (!this.settings.gui.enable_restapi && enableRestapi) {
          await restApiStore.startRestApi()
        }
        this.settings.gui.enable_restapi = enableRestapi
        await this._updateSettings()
      }
    },
    async updateEnableAutostart(enableAutostart: boolean) {
      if (this.settings !== null) {
        this.settings.gui.enable_autostart = enableAutostart
        await this._updateSettings()
      }
    },
    async updateRestapiPort(restapiPort: number) {
      if (this.settings !== null) {
        const restApiStore = useRestApiStore()
        const changed = this.settings.restapi?.port != restapiPort
        if (changed && (restApiStore.starting || restApiStore.started)) {
          await restApiStore.stopRestApi()
        }
        if (this.settings.restapi) {
          this.settings.restapi.port = restapiPort
        }
        await this._updateSettings()
        if (changed) {
          await restApiStore.startRestApi()
        }
      }
    },
    async openInBrowser(url: string) {
      await invoke('open_in_browser', {url})
    },
    async updateClientCount(clientCount: number) {
      if (this.settings !== null) {
        this.settings.factorio.client_count = clientCount
        await this._updateSettings()
      }
    },
    async updateMapExchangeString(maxExchangeString: string) {
      if (this.settings !== null) {
        this.settings.factorio.map_exchange_string = maxExchangeString
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
