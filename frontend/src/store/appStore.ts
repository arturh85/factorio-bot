import { defineStore } from 'pinia'

export const useAppStore = defineStore({
  id: 'app',
  state: () => ({
    foo: 1,
  }),
  getters: {
    getFoo(): number {
      return this.foo
    },
  },
})
