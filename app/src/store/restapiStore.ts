import {defineStore} from 'pinia'
import {invoke} from '@tauri-apps/api/tauri';
import {useAppStore} from '@/store/appStore';

export const useRestApiStore = defineStore({
    id: 'restapi',
    state: () => ({
        starting: false,
        stopping: false,
        started: false,
        failed: false,
        portAvailable: true
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
        },
        isPortAvailable(): boolean {
            return this.portAvailable
        }
    },
    actions: {
        async init() {
            this.started = await invoke('is_restapi_started')
        },
        async startRestApi() {
            if(this.started) {
                throw new Error('already started')
            }
            const appStore = useAppStore()
            this.portAvailable = await invoke('is_port_available', {port: appStore.settings?.restapi?.port || 0 })
            if (!this.portAvailable) {
                return
            }

            this.failed = false
            this.starting = true
            try {
                await invoke('start_restapi')
                this.starting = false
                this.started = true
            } catch(err) {
                console.error('failed to start rest api', err)
                this.starting = false
                this.failed = true
                throw new Error(err as string)
            }
        },
        async stopRestApi() {
            if(!this.started) {
                throw new Error('not started')
            }
            this.failed = false
            this.stopping = true
            try {
                await invoke('stop_restapi')
                this.stopping = false
                this.started = false
            } catch(err) {
                console.error('failed to stop rest api', err)
                this.stopping = false
                this.failed = true
                throw new Error(err as string)
            }
        }
    }
})
