Factorio Bot
============

This project tries to be a bot platform for the game
[Factorio](https://www.factorio.com) (version 1.0) inspired by [factorio-bot](https://github.com/Windfisch/factorio-bot/)

Features:
- extract factorio .zip/tar.xz and symlink bridge mod
- start Factorio server and/or one or multiple clients (unrestricted) 
- read factorio recipes/entity prototypes/item prototypes/graphics
- read map contents by chunk for leaflet based Map View
- Build Graphs of:
  - Entity Connections with distance based weights
  - Flow Connections with flow rate per second for each belt side/resource
  - Bot Task Dependencies with time estimate based weights 
- Use whatever mods you want
- REST Interface with:
  - retrieve all recipes/technologies/prototypes
  - read bot inventories
  - use bot to:
    - walk somewhere
    - mine something
    - craft something
    - place entities
    - place blueprints (even partly)
    - revive ghost entities
  - find valid placement options for:
    - offshore pumps
    - miners with specific ore below
    - blueprints (todo)
  - A* based belt/pipe routing from point a to b with automatic underground usage
  - parse blueprint strings into width/height + entity list
  - transfer items between bots
  - read entity inventories
  - start research
  - find entities/tiles in circle / rect
  - insert to inventory / remove from inventory
  - cheating variants of above methods for quick tests
- WebSocket based Events when:
  - bot changed position
  - bot changed inventory
  - research finished
  - entity attacked by biters (only an idea)
- Typescript based Bot Manager which can:
  - Build starter base with iron/copper/stone/coal burner-mining-drills and stone-furnaces
  - Start automation research & manually craft 10 `automation-science` to insert into `lab`
  - Start logistics research & build assembler to automatically craft the rest
- should work on Win/Mac/Linux, not tested on Mac
- MIT licenced

## Youtube Videos

- [Any% World Record gets automation in 7:33](https://www.youtube.com/watch?v=rHvaZMdjnLE&t=455) 

- [Factorio Bot 0.1.2: Research logistics with 4 Bots in 15:51](https://youtu.be/iFhcyjfcjx8) 
- [Factorio Bot 0.1.1: Research automation with 1 Bot in 8:57](https://youtu.be/1vbWWiSV6Sw) 
- [Factorio Bot 0.1.0: Research automation with 1 Bot in 12:33](https://youtu.be/6KXYuVDRZ-I) 

## Setup 

- Install [rust/cargo](https://rustup.rs/) and [nodejs/npm](https://nodejs.org/)
- Clone this repository
- Download [Factorio](https://www.factorio.com) as .zip or .tar.xz into workspace/ directory (don't use the headless version!)
- Start server & one client with good seed:

```
cargo run -- start -c 1 --new --seed 1785882545 
```

You can also use map-exchange-strings like this:

```
cargo run -- start -c 1 --new --seed 1759324908 --map ">>>eNpjZICDBnsQycGSnJ+YA+EdcABhruT8goLUIt38olRkYc7ko tKUVN38TFTFqXmpuZW6SYnFqTATQTRHZlF+HroJrMUl+XmoIiVFq anFDAwODqtXrbIDyXCXFiXmZZbmoutlYHyzT+hBQ4scAwj/r2dQ+ P8fhIGsB0AbQZiBsQGsgxEoBgUsEsn5eSVF+Tm6xaklJZl56VaJp RVWSZmJxZy6BnrGpgZAoIFNSVpRamFpal5ypVVuaU5JZkFOZmoRh 7GeARjIouvIzc8sLiktSgWbzGGgBzbXQBenMqymG+gZmgGBOWtyT mZaGgODgiMQO4H9xcBYLbLO/WHVFHtGiL/0HKCMD1CRA0kwEU8Yw 88Bp5QKjGGCZI4xGHxGYkAsLQFaAVXF4YBgQCRbQJKMjL1vty74f uyCHeOflR8v+SYl2DMauoq8+2C0zg4oyQ7yAhOcmDUTBHbCvMIAM /OBPVTqpj3j2TMg8MaekRWkQwREOFgAiQPezAyMAnxA1oIeIKEgw wBzmh3MGBEHxjQw+AbzyWMY47I9uj+AAWEDMlwORJwAEWAL4S5jh DAd+h0YHeRhspIIJUD9RgzIbkhB+PAkzNrDSPajOQQzIpD9gSai4 oAlGrhAFqbAiRfMcNcAw/MCO4znMN+BkRnEAKn6AhSD8EAyMKMgt IADM6KEACYLBvnZRmoATpjh0w==<<<"
```

See cargo run -- --help for other options.
On Windows the first start needs Administrative Privileges to create the symlinks to the mod directory.

## Docker 

There is also a docker image which starts the bot and factorio server in headless mode:

```
docker run --rm -p 34197:34197 -p 34197:34197/udp -p 7123:7123 arturh85/factorio-bot-rs
``` 

You can then connect to this headless server from different hosts with 

```
cargo run -- start --server <server-ip>
``` 


## Roll Best Seed for Map Exchange String:

```
docker run --rm arturh85/factorio-bot-rs roll-seed -p 5 -r 50 --map ">>>eNpjZICDBnsQycGSnJ+YA+EdcABhruT8goLUIt38olRkYc7ko tKUVN38TFTFqXmpuZW6SYnFqTATQTRHZlF+HroJrMUl+XmoIiVFq anFDAwODqtXrbIDyXCXFiXmZZbmoutlYHyzT+hBQ4scAwj/r2dQ+ P8fhIGsB0AbQZiBsQGsgxEoBgUsEsn5eSVF+Tm6xaklJZl56VaJp RVWSZmJxZy6BnrGpgZAoIFNSVpRamFpal5ypVVuaU5JZkFOZmoRh 7GeARjIouvIzc8sLiktSgWbzGGgBzbXQBenMqymG+gZmgGBOWtyT mZaGgODgiMQO4H9xcBYLbLO/WHVFHtGiL/0HKCMD1CRA0kwEU8Yw 88Bp5QKjGGCZI4xGHxGYkAsLQFaAVXF4YBgQCRbQJKMjL1vty74f uyCHeOflR8v+SYl2DMauoq8+2C0zg4oyQ7yAhOcmDUTBHbCvMIAM /OBPVTqpj3j2TMg8MaekRWkQwREOFgAiQPezAyMAnxA1oIeIKEgw wBzmh3MGBEHxjQw+AbzyWMY47I9uj+AAWEDMlwORJwAEWAL4S5jh DAd+h0YHeRhspIIJUD9RgzIbkhB+PAkzNrDSPajOQQzIpD9gSai4 oAlGrhAFqbAiRfMcNcAw/MCO4znMN+BkRnEAKn6AhSD8EAyMKMgt IADM6KEACYLBvnZRmoATpjh0w==<<<"
``` 

Once I upload my factorio mod to official mod portal you should be able to connect with any factorio client and have the mods auto sync.

## Contribute

Send me your Pull Requests :)

## Contact

Email: [arturh@arturh.de](mailto:arturh@arturh.de)
