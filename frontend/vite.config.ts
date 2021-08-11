import {defineConfig} from 'vite';
import vue from '@vitejs/plugin-vue';
import visualizer from 'rollup-plugin-visualizer'


const prefix = 'monaco-editor/esm/vs'
import * as path from 'path';

export default defineConfig({
    plugins: [
        vue(),
        visualizer({
            title: 'Bundle Size Visualizer',
            filename: 'dist/stats.html',
            template: 'treemap',
            brotliSize: true
        })
    ],
    resolve: {
        alias: {
            '@': path.resolve(__dirname, './src')
            // 'vue-i18n': 'vue-i18n/dist/vue-i18n.cjs.js'
        }
    },
    build: {
        rollupOptions: {
            output: {
                manualChunks: {
                    jsonWorker: [`${prefix}/language/json/json.worker`],
                    cssWorker: [`${prefix}/language/css/css.worker`],
                    htmlWorker: [`${prefix}/language/html/html.worker`],
                    tsWorker: [`${prefix}/language/typescript/ts.worker`],
                    editorWorker: [`${prefix}/editor/editor.worker`]
                }
            }
        }
    },
    define: {
        'process.env': {}
    }
});
