use crate::graph::task_graph::{TaskData, TaskStatus};
use crate::plan::planner::Planner;
use futures::future::join_all;
use miette::Result;
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use std::time::Duration;

#[allow(dead_code)]
async fn execute(planner: &Planner) -> Result<()> {
    let results = join_all(
        planner
            .plan_world
            .players
            .iter()
            .map(|f| execute_single(planner, f.player_id)),
    )
    .await;

    // Return first error if any bot failed
    for result in results {
        result?;
    }
    Ok(())
}

#[allow(dead_code, clippy::await_holding_lock)]
async fn execute_single(planner: &Planner, player_id: u8) -> Result<()> {
    let graph = planner.graph.read();
    let mut cursor = graph.start_node;

    while cursor != graph.end_node {
        let node = graph
            .node_weight(cursor)
            .ok_or_else(|| miette::miette!("Invalid node index: {:?}", cursor))?;

        // Check if all incoming dependencies are satisfied
        let mut dependencies_satisfied = false;
        while !dependencies_satisfied {
            dependencies_satisfied = true;

            for edge in graph.edges_directed(cursor, Direction::Incoming) {
                let source_idx = edge.source();

                // Get the source node
                let source_node = graph
                    .node_weight(source_idx)
                    .ok_or_else(|| miette::miette!("Invalid source node: {:?}", source_idx))?;

                // Skip structural nodes (start, group markers)
                if source_node.player_id.is_none() {
                    continue;
                }

                // Check if source task is completed
                let source_status = source_node.status.read();
                match *source_status {
                    TaskStatus::Success(_, _) => {
                        // Dependency satisfied
                        continue;
                    }
                    TaskStatus::Failed(_, _, ref msg) => {
                        // Dependency failed, can't proceed
                        return Err(miette::miette!(
                            "Dependency task '{}' failed: {}",
                            source_node.name,
                            msg
                        ));
                    }
                    _ => {
                        // Dependency not yet complete - need to wait
                        dependencies_satisfied = false;
                        break;
                    }
                }
            }

            // If dependencies not satisfied, wait before re-checking
            if !dependencies_satisfied {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        if let Some(data) = &node.data {
            // Transition to Running state
            {
                let mut status = node.status.write();
                match *status {
                    TaskStatus::Planned(_) => {
                        *status = TaskStatus::Running(0, 0); // TODO: real ticks in Phase 2.3
                    }
                    _ => continue, // Already executed, skip
                }
            }

            // Execute the task
            let execution_result: Result<()> = async {
                match data {
                    TaskData::Mine(target) => {
                        planner
                            .rcon
                            .as_ref()
                            .ok_or_else(|| miette::miette!("RCON connection not available"))?
                            .player_mine(
                                &planner.real_world,
                                player_id,
                                &target.name,
                                &target.position,
                                target.count,
                            )
                            .await?;
                    }
                TaskData::Walk(target) => {
                    planner
                        .rcon
                        .as_ref()
                        .ok_or_else(|| miette::miette!("RCON connection not available"))?
                        .move_player(
                            &planner.real_world,
                            player_id,
                            &target.position,
                            Some(target.radius),
                        )
                        .await?;
                }
                TaskData::Craft(item) => {
                    planner
                        .rcon
                        .as_ref()
                        .ok_or_else(|| miette::miette!("RCON connection not available"))?
                        .player_craft(&planner.real_world, player_id, &item.name, item.count)
                        .await?;
                }
                TaskData::InsertToInventory(location, item) => {
                    planner
                        .rcon
                        .as_ref()
                        .ok_or_else(|| miette::miette!("RCON connection not available"))?
                        .insert_to_inventory(
                            player_id,
                            location.entity_name.clone(),
                            location.position.clone(),
                            location.inventory_type,
                            item.name.clone(),
                            item.count,
                            &planner.real_world,
                        )
                        .await?;
                }
                TaskData::RemoveFromInventory(location, item) => {
                    planner
                        .rcon
                        .as_ref()
                        .ok_or_else(|| miette::miette!("RCON connection not available"))?
                        .remove_from_inventory(
                            player_id,
                            location.entity_name.clone(),
                            location.position.clone(),
                            location.inventory_type,
                            item.name.clone(),
                            item.count,
                            &planner.real_world,
                        )
                        .await?;
                }
                TaskData::PlaceEntity(entity) => {
                    planner
                        .rcon
                        .as_ref()
                        .ok_or_else(|| miette::miette!("RCON connection not available"))?
                        .place_entity(
                            player_id,
                            entity.name.clone(),
                            entity.position.clone(),
                            entity.direction,
                            &planner.real_world,
                        )
                        .await?;
                }
                }
                Ok(())
            }
            .await;

            // Handle result and update status
            match execution_result {
                Ok(_) => {
                    // Transition to Success state
                    let mut status = node.status.write();
                    *status = TaskStatus::Success(0.0, 0); // TODO: track real time in Phase 2.3
                }
                Err(e) => {
                    // Transition to Failed state
                    let mut status = node.status.write();
                    *status = TaskStatus::Failed(0, 0, format!("{:?}", e));
                    // Return error to stop execution for this bot
                    return Err(e);
                }
            }
        }

        let cursor_copy = cursor;
        for edge in graph.edges_directed(cursor, Direction::Outgoing) {
            let target_idx = edge.target();
            let target = graph
                .node_weight(target_idx)
                .ok_or_else(|| miette::miette!("Invalid target node: {:?}", target_idx))?;
            if target.player_id.is_none() || target.player_id.unwrap() == player_id {
                cursor = target_idx;
            }
        }
        if cursor == cursor_copy {
            error!("no change in cursor!?");
            break;
        }
    }
    Ok(())
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
                .expect_move_player()
                .times(player_count as usize)
                .returning(|_, _, _, _| Ok(()));
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
