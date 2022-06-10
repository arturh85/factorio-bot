import {FactorioSettings} from '@/models/types';

export type AppSettings = {
    gui: GuiSettings,
    restapi: any,
    factorio: FactorioSettings,
}


export type GuiSettings = {
    enable_autostart: boolean,
    enable_restapi: boolean,
}