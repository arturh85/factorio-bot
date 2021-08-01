import {defineStore} from 'pinia'
import {invoke} from '@tauri-apps/api/tauri';

export const useInstanceStore = defineStore({
    id: 'instance',
    state: () => ({}),
    actions: {
        async startInstances() {
            await invoke('start_instances')
        }
    }
})
