import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {toastMessages, useToast} from './useToast';

beforeEach(() => {
    vi.useFakeTimers();
    useToast().clear();
});

afterEach(() => {
    useToast().clear();
    vi.useRealTimers();
});

describe('useToast', () => {
    it('keeps a message until its life expires, then drops it', () => {
        useToast().add({severity: 'success', summary: 'Started!', life: 1000});
        vi.advanceTimersByTime(999);
        expect(toastMessages.length).toBe(1);
        vi.advanceTimersByTime(1);
        expect(toastMessages.length).toBe(0);
    });

    it('defaults severity to info and detail to null', () => {
        useToast().add({summary: 'Info Message'});
        expect(toastMessages[0].severity).toBe('info');
        expect(toastMessages[0].detail).toBeNull();
    });

    it('keeps a message with life 0 on screen indefinitely', () => {
        useToast().add({summary: 'sticky', life: 0});
        vi.advanceTimersByTime(600000);
        expect(toastMessages.length).toBe(1);
    });

    it('removes only the message asked for', () => {
        const first = useToast().add({summary: 'first', life: 0});
        useToast().add({summary: 'second', life: 0});
        useToast().remove(first);
        expect(toastMessages.map(message => message.summary)).toEqual(['second']);
    });

    it('cancels pending timers on clear, so a later message is not dropped early', () => {
        useToast().add({summary: 'first', life: 1000});
        useToast().clear();
        vi.advanceTimersByTime(1000);
        useToast().add({summary: 'second', life: 5000});
        vi.advanceTimersByTime(1000);
        // Without clearTimeout in clear(), the first message's timer fires at
        // t=1000 and splices index 0 -- which by then is 'second'.
        expect(toastMessages.map(message => message.summary)).toEqual(['second']);
    });
});
