import js from '@eslint/js'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import tseslint from 'typescript-eslint'
import { defineConfig, globalIgnores } from 'eslint/config'

export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      js.configs.recommended,
      tseslint.configs.recommended,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
    },
    rules: {
      'react-refresh/only-export-components': [
        'warn',
        { allowConstantExport: true },
      ],
      // Raw useForm's setValue silently skips dirty/touched/validation state,
      // so non-text inputs (Switch/Select/Checkbox wired via watch+setValue)
      // never enable dirty-gated submit buttons. useAppForm flips those
      // defaults; see src/components/ui/form.tsx.
      'no-restricted-imports': [
        'error',
        {
          paths: [
            {
              name: 'react-hook-form',
              importNames: ['useForm'],
              message:
                'Use useAppForm from "@/components/ui/form" instead: its setValue defaults to shouldDirty/shouldTouch/shouldValidate so non-text inputs mark the form dirty.',
            },
          ],
        },
      ],
    },
  },
  {
    // The one place allowed to wrap the raw hook.
    files: ['src/components/ui/form.tsx'],
    rules: {
      'no-restricted-imports': 'off',
    },
  },
  {
    files: ['src/main.tsx'],
    rules: {
      'react-refresh/only-export-components': 'off',
    },
  },
])
