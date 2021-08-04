import {defineStore} from 'pinia'
import {invoke} from '@tauri-apps/api/tauri';

export const useScriptStore = defineStore({
    id: 'script',
    state: () => ({
        code: '',
        executing: false,
        success: false,
        error: false
    }),
    getters: {
        getCode(): string {
            return this.code
        }
    },
    actions: {
        setCode(code: string) {
            this.code = code
        },
        async execute() {
            if(!this.code) {
                throw new Error('no code to execute?')
            }
            this.error = false
            this.executing = true
            try {
                await invoke('execute_script', {code: this.code})
                this.executing = false
                this.success = true
            } catch(err) {
                console.error('failed to execute script', err)
                this.executing = false
                this.success = true
                this.error = true
            }
        }
    }
})
