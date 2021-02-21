use std::time::Duration;

use actix::prelude::*;
use actix_web_actors::ws;
use actix_web_actors::ws::ProtocolError;
use serde_json::Value;

use crate::types::{
    FactorioPlayer, PlayerChangedDistanceEvent, PlayerChangedMainInventoryEvent,
    PlayerChangedPositionEvent, PlayerLeftEvent,
};

pub struct FactorioWebSocketClient {}

impl Actor for FactorioWebSocketClient {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<Result<ws::Message, ProtocolError>> for FactorioWebSocketClient {
    fn handle(&mut self, _result: Result<ws::Message, ProtocolError>, _ctx: &mut Self::Context) {
        // match result {
        //     Ok(msg) => {
        //         println!("WS: {:?}", msg);
        //         match msg {
        //             Message::Ping(msg) => {
        //                 ctx.pong(&msg);
        //             }
        //             Message::Pong(_) => {}
        //             Message::Text(text) => ctx.text(text),
        //             Message::Binary(bin) => ctx.binary(bin),
        //             Message::Close(_) => {
        //                 ctx.stop();
        //             }
        //             Message::Nop => (),
        //             Message::Continuation(_) => {}
        //         }
        //     }
        //     Err(err) => panic!("error"),
        // }
    }
}

impl Handler<ServerEvent> for FactorioWebSocketClient {
    type Result = ();

    fn handle(&mut self, msg: ServerEvent, ctx: &mut Self::Context) {
        ctx.text(msg.event);
    }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct RegisterWSClient {
    pub addr: Addr<FactorioWebSocketClient>,
}

#[derive(Message)]
#[rtype(result = "()")]
struct ServerEvent {
    event: String,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PlayerChangedPositionMessage {
    pub player: FactorioPlayer,
}
#[derive(Message)]
#[rtype(result = "()")]
pub struct PlayerDistanceChangedMessage {
    pub player: FactorioPlayer,
}
#[derive(Message)]
#[rtype(result = "()")]
pub struct PlayerChangedMainInventoryMessage {
    pub player: FactorioPlayer,
}
#[derive(Message)]
#[rtype(result = "()")]
pub struct PlayerLeftMessage {
    pub player_id: u32,
}
#[derive(Message)]
#[rtype(result = "()")]
pub struct ResearchCompletedMessage {}

#[derive(Message, Serialize)]
#[rtype(result = "()")]
pub struct TaskStarted {
    pub node_id: usize,
    pub tick: u32,
}
#[derive(Message, Serialize)]
#[rtype(result = "()")]
pub struct TaskSuccess {
    pub node_id: usize,
    pub tick: u32,
    pub duration: u32,
}

#[derive(Message, Serialize)]
#[rtype(result = "()")]
pub struct TaskFailed {
    pub node_id: usize,
    pub tick: u32,
    pub duration: u32,
    pub error: String,
}

pub struct FactorioWebSocketServer {
    pub listeners: Vec<Addr<FactorioWebSocketClient>>,
}

impl Actor for FactorioWebSocketServer {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.run_interval(Duration::from_secs(60), |act, _| {
            for l in &act.listeners {
                l.do_send(ServerEvent {
                    event: String::from("Heartbeat"),
                });
            }
        });
    }
}

impl FactorioWebSocketServer {
    fn broadcast(&mut self, event: Value) {
        for l in &self.listeners {
            l.do_send(ServerEvent {
                event: serde_json::to_string(&event).expect("failed to serialize"),
            });
        }
    }
}

impl Handler<RegisterWSClient> for FactorioWebSocketServer {
    type Result = ();

    fn handle(&mut self, msg: RegisterWSClient, _: &mut Context<Self>) {
        self.listeners.push(msg.addr);
    }
}

impl Handler<PlayerChangedPositionMessage> for FactorioWebSocketServer {
    type Result = ();

    fn handle(&mut self, msg: PlayerChangedPositionMessage, _: &mut Context<Self>) {
        self.broadcast(json!([
            "updatePlayerPosition",
            serde_json::to_value(PlayerChangedPositionEvent {
                player_id: msg.player.player_id,
                position: msg.player.position
            })
            .expect("failed to serialize")
        ]));
    }
}

impl Handler<PlayerChangedMainInventoryMessage> for FactorioWebSocketServer {
    type Result = ();

    fn handle(&mut self, msg: PlayerChangedMainInventoryMessage, _: &mut Context<Self>) {
        self.broadcast(json!([
            "updatePlayerMainInventory",
            serde_json::to_value(PlayerChangedMainInventoryEvent {
                player_id: msg.player.player_id,
                main_inventory: msg.player.main_inventory
            })
            .expect("failed to serialize")
        ]));
    }
}

impl Handler<PlayerDistanceChangedMessage> for FactorioWebSocketServer {
    type Result = ();

    fn handle(&mut self, msg: PlayerDistanceChangedMessage, _: &mut Context<Self>) {
        self.broadcast(json!([
            "updatePlayerDistance",
            serde_json::to_value(PlayerChangedDistanceEvent {
                player_id: msg.player.player_id,
                build_distance: msg.player.build_distance,
                reach_distance: msg.player.reach_distance,
                drop_item_distance: msg.player.drop_item_distance,
                item_pickup_distance: msg.player.item_pickup_distance,
                loot_pickup_distance: msg.player.loot_pickup_distance,
                resource_reach_distance: msg.player.resource_reach_distance,
            })
            .expect("failed to serialize")
        ]));
    }
}

impl Handler<PlayerLeftMessage> for FactorioWebSocketServer {
    type Result = ();

    fn handle(&mut self, msg: PlayerLeftMessage, _: &mut Context<Self>) {
        self.broadcast(json!([
            "playerLeft",
            serde_json::to_value(PlayerLeftEvent {
                player_id: msg.player_id,
            })
            .expect("failed to serialize")
        ]));
    }
}

impl Handler<ResearchCompletedMessage> for FactorioWebSocketServer {
    type Result = ();

    fn handle(&mut self, _msg: ResearchCompletedMessage, _: &mut Context<Self>) {
        self.broadcast(json!(["researchCompleted",]));
    }
}

impl Handler<TaskStarted> for FactorioWebSocketServer {
    type Result = ();

    fn handle(&mut self, _msg: TaskStarted, _: &mut Context<Self>) {
        self.broadcast(json!(["task", "started", _msg]));
    }
}

impl Handler<TaskSuccess> for FactorioWebSocketServer {
    type Result = ();

    fn handle(&mut self, _msg: TaskSuccess, _: &mut Context<Self>) {
        self.broadcast(json!(["task", "success", _msg]));
    }
}

impl Handler<TaskFailed> for FactorioWebSocketServer {
    type Result = ();

    fn handle(&mut self, _msg: TaskFailed, _: &mut Context<Self>) {
        self.broadcast(json!(["task", "failed", _msg]));
    }
}
