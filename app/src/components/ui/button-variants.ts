import {cva, type VariantProps} from 'class-variance-authority';

/**
 * Kept out of Button.vue because an SFC's <script setup> cannot export a
 * second binding, and pages occasionally want the class string on an <a>.
 */
export const buttonVariants = cva(
    'inline-flex cursor-pointer items-center justify-center gap-2 rounded-card font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus disabled:cursor-not-allowed disabled:opacity-60',
    {
        variants: {
            variant: {
                primary: 'bg-brand text-white hover:bg-brand-dark',
                success: 'bg-success text-white hover:bg-success-dark',
                danger: 'bg-danger text-white hover:bg-danger-dark',
                ghost: 'bg-transparent text-ink hover:bg-divider'
            },
            size: {
                default: 'h-9 px-4 text-sm',
                sm: 'h-8 px-3 text-xs',
                icon: 'size-9'
            }
        },
        defaultVariants: {
            variant: 'primary',
            size: 'default'
        }
    }
);

export type ButtonVariants = VariantProps<typeof buttonVariants>;
