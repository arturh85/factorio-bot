import {defineStore} from 'pinia'
import {fetchFactorioVersions} from "@/services/factorio-versions";

export const useFactorioVersionsStore = defineStore({
    id: 'app',
    state: () => ({
        loading: false,
        factorioVersions: [""],

    }),
    getters: {
        getFactorioVersions(): string[] {
            return this.factorioVersions
        },
    },
    actions: {
        async loadFactorioVersions() {
            this.factorioVersions = await fetchFactorioVersions()
            return this.factorioVersions
        },
    }
})
