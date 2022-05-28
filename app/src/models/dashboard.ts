
export type DashboardMenu = {
    label: string,
    items: DashboardMenu[],
    command: Function,
    url: string,
    icon: string,
    class: string,
    badge: string,
    style: string,
    to: string,
    separator: string,
    disabled: boolean,
    target: any,
}