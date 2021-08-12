import {defineStore} from 'pinia'
import {invoke} from '@tauri-apps/api/tauri';
import {PrimeVueTreeNode} from '@/models/types';

export const useScriptStore = defineStore({
    id: 'script',
    state: () => ({
        code: '',
        executing: false,
        success: false,
        error: false,
        stdout: '',
        stderr: '',

        loadingScriptsInDirectory: false
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
        },
        getLoadingScriptsInDirectory(): boolean {
            return this.loadingScriptsInDirectory
        }
    },
    actions: {
        async loadScriptsInDirectory(path: string): Promise<PrimeVueTreeNode[]> {
            this.loadingScriptsInDirectory = true
            try {
                const result = await invoke('load_scripts_in_directory', {path}) as PrimeVueTreeNode[]
                this.loadingScriptsInDirectory = false
                return result
            } catch(err) {
                this.loadingScriptsInDirectory = false
                throw err
            }
        },
        async loadScriptFile(path: string): Promise<string> {
            try {
                const result = await invoke('load_script', {path}) as string
                this.code = result
                return result
            } catch(err) {
                console.error('failed', err)
                throw err
            }
        },
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
