import 'vite';

declare module 'vite' {
  interface FSWatcher {
    ref(): this;
    unref(): this;
  }
}

declare module 'vitest/node_modules/vite' {
  interface FSWatcher {
    ref(): this;
    unref(): this;
  }
}
