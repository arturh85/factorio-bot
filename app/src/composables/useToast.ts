import {reactive, readonly} from 'vue';

export type ToastSeverity = 'success' | 'info' | 'warn' | 'error';

export interface ToastOptions {
    severity?: ToastSeverity;
    summary: string;
    detail?: string;
    /** Milliseconds before auto-dismissal. 0 keeps the message until removed. */
    life?: number;
}

export interface ToastMessage {
    id: number;
    severity: ToastSeverity;
    summary: string;
    detail: string | null;
}

/**
 * One queue for the whole app, deliberately module-level.
 *
 * PrimeVue's useToast() resolved a service through Vue's injection, which
 * meant it only worked inside setup(). This app calls it from route guards
 * and from async callbacks that outlive the component, so the queue lives in
 * the module and any caller anywhere gets the same one.
 */
const messages = reactive([] as ToastMessage[]);
const timers = new Map<number, ReturnType<typeof setTimeout>>();
let nextId = 1;

export const toastMessages = readonly(messages);

function remove(id: number): void {
    const timer = timers.get(id);
    if (timer !== undefined) {
        clearTimeout(timer);
        timers.delete(id);
    }
    const index = messages.findIndex(message => message.id === id);
    if (index !== -1) {
        messages.splice(index, 1);
    }
}

function add(options: ToastOptions): number {
    const id = nextId++;
    messages.push({
        id,
        severity: options.severity ?? 'info',
        summary: options.summary,
        detail: options.detail ?? null
    });
    const life = options.life ?? 5000;
    if (life > 0) {
        timers.set(id, setTimeout(() => remove(id), life));
    }
    return id;
}

function clear(): void {
    for (const timer of timers.values()) {
        clearTimeout(timer);
    }
    timers.clear();
    messages.splice(0, messages.length);
}

export function useToast() {
    return {add, remove, clear};
}
