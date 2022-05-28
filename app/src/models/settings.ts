import {FactorioSettings, RestApiSettings} from '@/models/types';

export type AppSettings = {
    gui: GuiSettings,
    restapi: RestApiSettings,
    factorio: FactorioSettings,
}


export type GuiSettings = {
    enable_autostart: boolean,
    enable_restapi: boolean,
}