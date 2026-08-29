import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue';
import visualizer from 'rollup-plugin-visualizer'
import * as path from 'path';

export default defineConfig({
    plugins: [
        vue(),
        visualizer({
            title: 'Bundle Size Visualizer',
            filename: 'dist/stats.html',
            template: 'treemap',
            brotliSize: true
        }) as any
    ],
    test: {
        coverage: {
            reporter: ['html-spa', 'cobertura']
        }
    },
    resolve: {
        alias: {
            '@': path.resolve(__dirname, './src')
            // 'vue-i18n': 'vue-i18n/dist/vue-i18n.cjs.js'
        }
    },
    define: {
        'process.env': {}
    }
});
