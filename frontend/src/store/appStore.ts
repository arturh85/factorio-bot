import { defineStore } from 'pinia'
import {AppSettings} from "@/models/types";
import {invoke} from "@tauri-apps/api/tauri";

export const useAppStore = defineStore({
  id: 'app',
  state: () => ({
    settings: null,

  }),
  getters: {
    getSettings(): AppSettings | null {
      return this.settings
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
