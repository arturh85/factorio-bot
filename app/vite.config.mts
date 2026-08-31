import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue';
import { visualizer } from 'rollup-plugin-visualizer'
import tailwindcss from '@tailwindcss/vite'
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';

// The axum backend serves both the SPA and the API from one origin in
// production, but `vite --port 8080` (the `serve` script) is a second origin
// with nothing behind `/api` or `/openapi.json` unless we forward it. This is
// the alternative to `VITE_API_BASE`, not a replacement for it: `apiBase()` in
// `src/api/http.ts` prefers that env var when set, making requests absolute
// and bypassing this proxy entirely -- useful when the backend isn't
// reachable at this default. Leave `VITE_API_BASE` unset to go through the
// proxy below instead.
const backendTarget = 'http://127.0.0.1:7492';

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
    server: {
        port: 8080,
        proxy: {
            '/api': {
                target: backendTarget,
                changeOrigin: true
            },
            '/openapi.json': {
                target: backendTarget,
                changeOrigin: true
            },
            '/swagger-ui': {
                target: backendTarget,
                changeOrigin: true
            }
        }
    },
    test: {
        environment: 'node',
        coverage: {
            reporter: ['html-spa', 'cobertura', 'text'],
            // The transport swap's blast radius, and nothing else. Views and
            // routing were untested before this change and are out of scope
            // here; widening `include` without writing the tests first would
            // just move the thresholds down to meaninglessness.
            include: ['src/api/**/*.ts', 'src/store/**/*.ts'],
            exclude: ['**/*.spec.ts'],
            thresholds: {
                lines: 90,
                functions: 90,
                statements: 90,
                branches: 80
            }
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
