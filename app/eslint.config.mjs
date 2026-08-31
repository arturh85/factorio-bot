// Flat config (ESLint 10). Replaces .eslintrc.js + .eslintignore, neither of
// which ESLint 10 reads any more.
//
// .mjs rather than .js because package.json has no "type": "module", so a
// plain .js config would be loaded as CommonJS and the imports below would
// throw.
import { defineConfig, globalIgnores } from 'eslint/config';
import tseslint from 'typescript-eslint';
import pluginVue from 'eslint-plugin-vue';

export default defineConfig([
    // was .eslintignore; patterns resolve relative to this file's directory,
    // which is the same anchoring .eslintignore had
    globalIgnores(['**/*.d.ts']),

    // Order matters: typescript-eslint's base config carries no `files` key, so
    // it sets the parser for every file including .vue. The vue configs must
    // come after it to win the parser back for single-file components.
    ...tseslint.configs.recommended,
    ...pluginVue.configs['flat/essential'],

    {
        name: 'factorio-bot/language-options',
        files: ['**/*.js', '**/*.ts', '**/*.vue'],
        languageOptions: {
            ecmaVersion: 2020,
            sourceType: 'module',
            parserOptions: {
                ecmaFeatures: { jsx: true }
            }
        }
    },

    // vue's flat/base already installs vue-eslint-parser for **/*.vue; this
    // hands it @typescript-eslint/parser for <script lang="ts"> blocks.
    {
        name: 'factorio-bot/vue-script-parser',
        files: ['**/*.vue'],
        languageOptions: {
            parserOptions: {
                parser: tseslint.parser,
                ecmaVersion: 2020,
                sourceType: 'module',
                ecmaFeatures: { jsx: true }
            }
        }
    },

    {
        name: 'factorio-bot/rules',
        rules: {
            '@typescript-eslint/explicit-function-return-type': 'off',
            '@typescript-eslint/no-explicit-any': 'off',
            '@typescript-eslint/no-var-requires': 'off',
            '@typescript-eslint/no-empty-function': 'off',
            'vue/custom-event-name-casing': 'off',
            'no-use-before-define': 'off',
            '@typescript-eslint/no-use-before-define': 'off',
            '@typescript-eslint/ban-ts-comment': 'off',
            '@typescript-eslint/no-non-null-assertion': 'off',
            '@typescript-eslint/explicit-module-boundary-types': 'off',

            // The core rule must stay off while the TypeScript one is on:
            // running both double-reports every finding, and the core rule
            // flags type-only positions it cannot understand.
            'no-unused-vars': 'off',
            '@typescript-eslint/no-unused-vars': [
                'error',
                {
                    argsIgnorePattern: '^h$',
                    varsIgnorePattern: '^h$'
                }
            ],

            // Editor and Dashboard predate the rule and renaming them would
            // touch every import and template that references them. The
            // shadcn-style primitives under components/ui/ (Toaster, and more
            // arriving in later redesign tasks) are deliberately single-word,
            // matching the upstream shadcn/ui naming this redesign follows.
            'vue/multi-word-component-names': [
                'error',
                { ignores: ['Editor', 'Dashboard', 'Toaster', 'Button', 'Toggle', 'Input', 'Checkbox', 'Slider', 'Label', 'Card', 'Textarea'] }
            ],

            'space-before-function-paren': 'off',
            quotes: ['error', 'single'],
            'comma-dangle': ['error', 'never']
        }
    }
]);
