import {defineStore} from 'pinia'
import {fetchFactorioVersions} from "@/services/factorio-versions";

export const useFactorioVersionsStore = defineStore({
    id: 'factorioVersions',
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
            console.log('factorio versions', this.factorioVersions)
            return this.factorioVersions
        },
    }
})
