use std::borrow::Cow;

#[derive(
    Debug, Clone, typescript_definitions::TypeScriptify, serde::Serialize, serde::Deserialize,
)]
#[allow(non_camel_case_types)]
pub struct FactorioSettings {
    pub client_count: u8,
    pub factorio_archive_path: Cow<'static, str>,
    pub map_exchange_string: Cow<'static, str>,
    pub rcon_pass: Cow<'static, str>,
    pub rcon_port: u16,
    pub recreate: bool,
    pub restapi_port: u16,
    pub seed: Cow<'static, str>,
    pub workspace_path: Cow<'static, str>,
}

impl Default for FactorioSettings {
    fn default() -> Self {
        FactorioSettings {
            client_count: 2,
            factorio_archive_path: Cow::Borrowed(""),
            map_exchange_string: Cow::Borrowed(">>>eNpjZICDBnsQycGSnJ+YA+EdcABhruT8goLUIt38olRkYc7ko tKUVN38TFTFqXmpuZW6SYnFqTATQTRHZlF+HroJrMUl+XmoIiVFq anFDAwODqtXrbIDyXCXFiXmZZbmoutlYHyzT+hBQ4scAwj/r2dQ+ P8fhIGsB0AbQZiBsQGsgxEoBgUsEsn5eSVF+Tm6xaklJZl56VaJp RVWSZmJxZy6BnrGpgZAoIFNSVpRamFpal5ypVVuaU5JZkFOZmoRh 7GeARjIouvIzc8sLiktSgWbzGGgBzbXQBenMqymG+gZmgGBOWtyT mZaGgODgiMQO4H9xcBYLbLO/WHVFHtGiL/0HKCMD1CRA0kwEU8Yw 88Bp5QKjGGCZI4xGHxGYkAsLQFaAVXF4YBgQCRbQJKMjL1vty74f uyCHeOflR8v+SYl2DMauoq8+2C0zg4oyQ7yAhOcmDUTBHbCvMIAM /OBPVTqpj3j2TMg8MaekRWkQwREOFgAiQPezAyMAnxA1oIeIKEgw wBzmh3MGBEHxjQw+AbzyWMY47I9uj+AAWEDMlwORJwAEWAL4S5jh DAd+h0YHeRhspIIJUD9RgzIbkhB+PAkzNrDSPajOQQzIpD9gSai4 oAlGrhAFqbAiRfMcNcAw/MCO4znMN+BkRnEAKn6AhSD8EAyMKMgt IADM6KEACYLBvnZRmoATpjh0w==<<<"),
            rcon_pass: Cow::Borrowed("foobar"),
            rcon_port: 4321,
            recreate: false,
            restapi_port: 1234,
            seed: Cow::Borrowed(""),
            workspace_path: Cow::Borrowed(""),
        }
    }
}
