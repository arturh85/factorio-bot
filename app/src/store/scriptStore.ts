import {defineStore} from 'pinia'
import {invoke} from '@tauri-apps/api/tauri';
import {PrimeVueTreeNode} from '@/models/types';
import {languageFromPath} from '@/utils';

export const useScriptStore = defineStore({
    id: 'script',
    state: () => ({
        code: '',
        language: 'lua',
        executing: false,
        success: false,
        error: false,
        stdout: '',
        stderr: '',

        activeScriptPath: '',
        loadingScriptsInDirectory: false
    }),
    getters: {
        isExecuting(): boolean {
            return this.executing
        },
        getCode(): string {
            return this.code
        },
        getLanguage(): string {
            return this.language
        },
        getStdout(): string {
            return this.stdout
        },
        getStderr(): string {
            return this.stderr
        },
        getLoadingScriptsInDirectory(): boolean {
            return this.loadingScriptsInDirectory
        },
        getActiveScriptPath(): string {
            return this.activeScriptPath
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
                this.activeScriptPath = path
                const result = await invoke('load_script', {path}) as string
                this.code = result
                this.language = languageFromPath(path)
                return result
            } catch(err) {
                console.error('failed', err)
                throw err
            }
        },
        async setCode(code: string) {
            this.code = code
            await invoke('save_script', {code: this.code, path: this.activeScriptPath})
        },
        async executeCode() {
            if(!this.code) {
                throw new Error('no code to execute?')
            }
            this.stdout = ''
            this.stderr = ''
            this.error = false
            this.executing = true
            try {
                const outputs = await invoke('execute_code', {code: this.code, language: this.language}) as string[]
                this.stdout = outputs[0]
                this.stderr = outputs[1]
                this.executing = false
                this.success = true
            } catch(err) {
                console.error('failed to execute script', err)
                this.executing = false
                this.success = true
                this.error = true
                throw new Error(err as string)
            }
        },
        async executeScript() {
            if(!this.activeScriptPath) {
                throw new Error('no script to execute?')
            }
            this.stdout = ''
            this.stderr = ''
            this.error = false
            this.executing = true
            try {
                const outputs = await invoke('execute_script', {path: this.activeScriptPath}) as any
                this.stdout = outputs[0]
                this.stderr = outputs[1]
                this.executing = false
                this.success = true
            } catch(err) {
                console.error('failed to execute script', err)
                this.executing = false
                this.success = true
                this.error = true
                throw new Error(err as string)
            }
        }
    }
})
