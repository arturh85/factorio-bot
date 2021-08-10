import {defineStore} from 'pinia'
import {invoke} from '@tauri-apps/api/tauri';

export const useInstanceStore = defineStore({
    id: 'instance',
    state: () => ({
        starting: false,
        stopping: false,
        started: false,
        failed: false
    }),
    getters: {
        isStarting(): boolean {
            return this.starting
        },
        isStopping(): boolean {
            return this.stopping
        },
        isFailed(): boolean {
            return this.failed
        },
        isStarted(): boolean {
            return this.started
        }
    },
    actions: {
        async startInstances() {
            if(this.started) {
                throw new Error('already started')
            }
            this.failed = false
            this.starting = true
            try {
                await invoke('start_instances')
                this.starting = false
                this.started = true
            } catch(err) {
                console.error('failed to start instances', err)
                this.starting = false
                this.failed = true
                throw new Error(err)
            }
        },
        async stopInstances() {
            if(!this.started) {
                throw new Error('not started')
            }
            this.failed = false
            this.stopping = true
            try {
                await invoke('stop_instances')
                this.stopping = false
                this.started = false
            } catch(err) {
                console.error('failed to stop instances', err)
                this.stopping = false
                this.failed = true
                throw new Error(err)
            }
        }
    }
})
