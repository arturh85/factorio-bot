use crate::graph::task_graph::TaskData;
use crate::plan::planner::Planner;
use futures::future::join_all;
use miette::Result;
use petgraph::visit::EdgeRef;
use petgraph::Direction;

#[allow(dead_code)]
async fn execute(planner: &Planner) -> Result<()> {
    join_all(
        planner
            .plan_world
            .players
            .iter()
            .map(|f| execute_single(planner, f.player_id)),
    )
    .await;
    Ok(())
}

#[allow(dead_code, clippy::await_holding_lock)]
async fn execute_single(planner: &Planner, player_id: u8) {
    let graph = planner.graph.read();
    let mut cursor = graph.start_node;

    while cursor != graph.end_node {
        let node = graph
            .node_weight(cursor)
            .expect("NodeIndices should all be valid");
        if let Some(data) = &node.data {
            match data {
                TaskData::Mine(target) => {
                    planner
                        .rcon
                        .as_ref()
                        .expect("Rcon Connection failed")
                        .player_mine(
                            &planner.real_world,
                            player_id,
                            &target.name,
                            &target.position,
                            target.count,
                        )
                        .await
                        .expect("Failed to mine");
                }
                TaskData::Walk(_) => {}
                TaskData::Craft(_) => {}
                TaskData::InsertToInventory(_, _) => {}
                TaskData::RemoveFromInventory(_, _) => {}
                TaskData::PlaceEntity(_) => {}
            }
        }

        let cursor_copy = cursor;
        for edge in graph.edges_directed(cursor, Direction::Outgoing) {
            let target_idx = edge.target();
            let target = graph
                .node_weight(target_idx)
                .expect("NodeIndices should all be valid");
            if target.player_id.is_none() || target.player_id.unwrap() == player_id {
                cursor = target_idx;
            }
        }
        if cursor == cursor_copy {
            error!("no change in cursor!?");
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::factorio::rcon::MockFactorioRcon;
    use crate::plan::plan_builder::PlanBuilder;
    use crate::test_utils::fixture_world;
    use crate::types::Position;
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn test_execution() {
        let player_count = 2;
        let world = Arc::new(fixture_world());
        let mut mock_rcon = MockFactorioRcon::default();
        // mock config
        {
            mock_rcon
                .expect_player_mine()
                .times(player_count as usize)
                .returning(|_, _, _, _, _| Ok(()));
        }
        let mock_rcon = Arc::new(mock_rcon);

        let mut planner = Planner::new(world, Some(mock_rcon));
        let all_bots = planner.initiate_missing_players_with_default_inventory(player_count);
        planner.update_plan_world();
        let plan_builder = Arc::new(PlanBuilder::new(
            planner.graph.clone(),
            planner.plan_world.clone(),
        ));
        // plan code
        {
            plan_builder.group_start("Mine Stuff");
            for idx in &all_bots {
                plan_builder
                    .mine(
                        *idx,
                        Position::new(*idx as f64 * 10.0, 43.0),
                        "rock-huge",
                        1,
                    )
                    .expect("failed");
            }
            plan_builder.group_end();
        }
        let graph = planner.graph();
        assert_eq!(
            graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "Process Start" ]
    1 [ label = "Process End" ]
    2 [ label = "Start: Mine Stuff" ]
    3 [ label = "Walk to [10, 43]" ]
    4 [ label = "Mining rock-huge" ]
    5 [ label = "Walk to [20, 43]" ]
    6 [ label = "Mining rock-huge" ]
    7 [ label = "End" ]
    0 -> 2 [ label = "0" ]
    2 -> 3 [ label = "45" ]
    2 -> 5 [ label = "48" ]
    3 -> 4 [ label = "3" ]
    4 -> 7 [ label = "3" ]
    5 -> 6 [ label = "3" ]
    6 -> 7 [ label = "0" ]
    7 -> 1 [ label = "0" ]
}
"#,
        );
        execute(&planner).await.expect("failed to execute");
    }
}
