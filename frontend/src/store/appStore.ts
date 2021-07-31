import { defineStore } from 'pinia'
import {AppSettings} from "@/models/types";
import {invoke} from "@tauri-apps/api/tauri";

const initialSettings: AppSettings | null = null;

export const useAppStore = defineStore({
  id: 'app',
  state: () => ({
    settings: initialSettings,

  }),
  getters: {
    getSettings(): AppSettings | null {
      return this.settings
    },
    getWorkspacePath(): string | null {
      if (this.settings !== null) {
        return this.settings?.workspace_path
      } else {
        return null;
      }
    },
  },
  actions: {
    async loadSettings() {
      this.settings = await invoke('load_settings')
    },
    async saveSettings() {
      await invoke('save_config', {settings: this.settings})
    }
  }
})
