import { defineStore } from 'pinia'
import {AppSettings} from '@/models/types';
import {invoke} from '@tauri-apps/api/tauri';

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
        return this.settings.workspace_path
      } else {
        return null;
      }
    },
    getFactorioVersion(): string | null {
      if (this.settings) {
        return this.settings.factorio_version
      } else {
        return null;
      }
    },
    getClientCount(): string | null {
      if (this.settings) {
        return this.settings.client_count
      } else {
        return null;
      }
    }
  },
  actions: {
    async loadSettings() {
      this.settings = await invoke('load_settings')
    },
    async _updateSettings() {
      await invoke('update_settings', {settings: this.settings})
    },
    async updateWorkspacePath(workspacePath: string) {
      if (this.settings !== null) {
        this.settings.workspace_path = workspacePath
        await this._updateSettings()
      }
    },
    async updateFactorioVersion(factorioVersion: string) {
      if (this.settings !== null) {
        this.settings.factorio_version = factorioVersion
        await this._updateSettings()
      }
    },
    async updateClientCount(clientCount: number) {
      if (this.settings !== null) {
        this.settings.client_count = clientCount
        await this._updateSettings()
      }
    }
  }
})
