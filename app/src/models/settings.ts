import {FactorioSettings, RestApiSettings} from '@/api/types';

/**
 * The whole of `GET`/`PUT /api/v1/settings` --
 * `factorio_bot_core::app_settings::AppSettings`.
 *
 * `restapi` used to be `any`, which let a store read a field the server has
 * never sent. It is the generated `RestApiSettings` now, so the two agree by
 * construction.
 */
export type AppSettings = {
    gui: GuiSettings,
    restapi: RestApiSettings,
    factorio: FactorioSettings,
}

/**
 * Not generated: `GuiSettings` lives in `crates/core/src/app_settings.rs`
 * and does not derive `TypeScriptify`, so it is mirrored here by hand.
 */
export type GuiSettings = {
    enable_autostart: boolean,
    enable_restapi: boolean,
}
