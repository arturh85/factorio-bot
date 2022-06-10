export function languageFromPath(path: string): string {
    const last_dot = path.lastIndexOf('.')
    if (last_dot !== -1) {
        return path.substring(last_dot+1)
    } else {
        return ''
    }
}