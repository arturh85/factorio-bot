import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue';
import { visualizer } from 'rollup-plugin-visualizer'
import tailwindcss from '@tailwindcss/vite'
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';

export default defineConfig({
    plugins: [
        vue(),
        tailwindcss(),
        visualizer({
            title: 'Bundle Size Visualizer',
            filename: 'dist/stats.html',
            template: 'treemap',
            brotliSize: true
        })
    ],
    test: {
        coverage: {
            reporter: ['html-spa', 'cobertura']
        }
    },
    resolve: {
        alias: {
            '@': path.resolve(path.dirname(fileURLToPath(import.meta.url)), './src')
            // 'vue-i18n': 'vue-i18n/dist/vue-i18n.cjs.js'
        }
    },
    define: {
        'process.env': {}
    }
});
