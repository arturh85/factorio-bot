import {defineStore} from 'pinia'
import {invoke} from '@tauri-apps/api/tauri';

export const useScriptStore = defineStore({
    id: 'script',
    state: () => ({
        code: 'local example = 5 + 5\nexample = example + 2\nrcon.print(example)',
        executing: false,
        success: false,
        error: false,
        stdout: '',
        stderr: ''
    }),
    getters: {
        isExecuting(): boolean {
            return this.executing
        },
        getCode(): string {
            return this.code
        },
        getStdout(): string {
            return this.stdout
        },
        getStderr(): string {
            return this.stderr
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
            this.stdout = ''
            this.stderr = ''
            this.error = false
            this.executing = true
            try {
                const outputs = await invoke('execute_script', {code: this.code}) as any
                this.stdout = outputs[0]
                this.stderr = outputs[1]
                this.executing = false
                this.success = true
            } catch(err) {
                console.error('failed to execute script', err)
                this.executing = false
                this.success = true
                this.error = true
                throw new Error(err)
            }
        }
    }
})
