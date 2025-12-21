import {defineStore} from 'pinia'
import {invoke} from '@tauri-apps/api/core';

export const useRconStore = defineStore({
    id: 'rcon',
    state: () => ({
        executing: false,
        success: false,
        error: false
    }),
    getters: {
        isExecuting(): boolean {
            return this.executing
        }
    },
    actions: {
        async execute(command: string) {
            if(!command) {
                throw new Error('no command to execute?')
            }
            this.error = false
            this.executing = true
            try {
                await invoke('execute_rcon', {command})
                this.executing = false
                this.success = true
            } catch(err) {
                console.error('failed to execute rcon', err)
                this.executing = false
                this.success = true
                this.error = true
            }
        }
    }
})
