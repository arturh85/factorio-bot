use crate::graph::task_graph::TaskStatus;
use crate::plan::planner::Planner;
use crate::types::PlayerId;
use petgraph::visit::EdgeRef;
use petgraph::Direction;

struct MermaidGanttBuilder {
    title: String,
    date_format: String,
    axis_format: String,
    buffer: String,
}

impl MermaidGanttBuilder {
    fn new(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            date_format: "HH:mm:ss".to_owned(),
            axis_format: "%H:%M:%S".to_owned(),
            buffer: String::new(),
        }
    }

    fn build(&self) -> String {
        let mut output = "gantt\n".to_owned();
        output += &format!("    title {}\n", self.title);
        output += &format!("    dateFormat {}\n", self.date_format);
        output += &format!("    axisFormat {}\n", self.axis_format);
        output += &self.buffer;
        output
    }

    #[allow(dead_code)]
    fn date_format(mut self, date_format: &str) -> Self {
        self.date_format = date_format.to_owned();

        self
    }

    #[allow(dead_code)]
    fn axis_format(mut self, axis_format: &str) -> Self {
        self.axis_format = axis_format.to_owned();

        self
    }

    fn add_milestone(mut self, label: &str, name: &str, timestamp: &str, duration: f64) -> Self {
        let label = self.replace_colon(label);
        let name = self.replace_colon(name);
        let line = format!("    {label} : milestone, {name}, {timestamp},{duration}s\n");
        self.buffer += &line;

        self
    }

    fn add_section(mut self, label: &str) -> Self {
        let label = self.replace_colon(label);
        let line = format!("    section {label}\n");
        self.buffer += &line;

        self
    }

    fn replace_colon(&self, input: &str) -> String {
        input.to_owned().replace(':', "﹕")
    }

    fn add_action(mut self, label: &str, duration: f64, timestamp: Option<&str>) -> Self {
        let label = self.replace_colon(label);
        let line = if let Some(timestamp) = timestamp {
            format!("    {label} : {timestamp},{duration}s\n")
        } else {
            format!("    {label} : {duration}s\n")
        };
        self.buffer += &line;

        self
    }
}

fn duration_to_timestamp(duration: f64) -> String {
    let duration = duration as u64;
    let seconds = duration % 60;
    let minutes = (duration / 60) % 60;
    let hours = (duration / 60 / 60) % 60;

    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub fn to_mermaid_gantt(plan: &Planner, bot_ids: Vec<PlayerId>, title: &str) -> String {
    let mut builder = MermaidGanttBuilder::new(title);
    let graph = plan.graph.read();

    let total_runtime = plan.graph().shortest_path().expect("no path found");

    builder = builder.add_milestone("test", "m1", &duration_to_timestamp(total_runtime), 0.);
    // let milestone_by_index: HashMap<NodeIndex, String> = HashMap::new();

    for player_id in bot_ids {
        builder = builder.add_section(&format!("Bot {}", player_id));
        let mut cursor = graph.start_node;
        let mut last_weight: Option<f64> = None;

        while cursor != graph.end_node {
            let node = graph
                .node_weight(cursor)
                .expect("NodeIndices should all be valid");

            let status = node.status.read();
            match *status {
                TaskStatus::Planned(estimated) => {
                    builder = builder.add_action(
                        &node.name,
                        if estimated > 0. {
                            estimated
                        } else {
                            last_weight.unwrap_or(0.)
                        },
                        if cursor == graph.start_node {
                            Some("00:00:00")
                        } else {
                            None
                        },
                    );
                }
                TaskStatus::Success(estimated, _realtime) => {
                    builder = builder.add_action(
                        &node.name,
                        if estimated > 0. {
                            estimated
                        } else {
                            last_weight.unwrap_or(0.)
                        },
                        if cursor == graph.start_node {
                            Some("00:00:00")
                        } else {
                            None
                        },
                    );
                }
                _ => {}
            };

            let cursor_copy = cursor;
            for edge in graph.edges_directed(cursor, Direction::Outgoing) {
                let target_idx = edge.target();
                last_weight = Some(*edge.weight());
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

    builder.build()
}

#[cfg(test)]
mod tests {
    use crate::graph::task_graph::PositionRadius;
    use crate::plan::plan_builder::PlanBuilder;
    use crate::test_utils::fixture_world;
    use crate::types::Position;
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn test_mermaid_gantt() {
        let player_count = 2;
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
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
            plan_builder.group_start("Walk Around Stuff");
            for idx in &all_bots {
                plan_builder
                    .add_walk(*idx, PositionRadius::new(*idx as f64 * 10.0, 3.0, 1.))
                    .expect("failed");
                plan_builder
                    .add_walk(*idx, PositionRadius::new(*idx as f64 * 10.0, 63.0, 1.))
                    .expect("failed");
                plan_builder
                    .add_walk(*idx, PositionRadius::new(0., 0., 1.))
                    .expect("failed");
            }
            plan_builder.group_end();
        }
        // let graph = planner.graph();
        //         assert_eq!(
        //             graph.graphviz_dot(),
        //             r#"digraph {
        //     0 [ label = "Process Start" ]
        //     1 [ label = "Process End" ]
        //     2 [ label = "Start: Mine Stuff" ]
        //     3 [ label = "Walk to [10, 43]" ]
        //     4 [ label = "Mining rock-huge" ]
        //     5 [ label = "Walk to [20, 43]" ]
        //     6 [ label = "Mining rock-huge" ]
        //     7 [ label = "End" ]
        //     0 -> 2 [ label = "0" ]
        //     2 -> 3 [ label = "45" ]
        //     2 -> 5 [ label = "48" ]
        //     3 -> 4 [ label = "3" ]
        //     4 -> 7 [ label = "3" ]
        //     5 -> 6 [ label = "3" ]
        //     6 -> 7 [ label = "0" ]
        //     7 -> 1 [ label = "0" ]
        // }
        // "#,
        //         );
        assert_eq!(
            to_mermaid_gantt(&planner, all_bots, "Example diagram"),
            r#"gantt
    title Example diagram
    dateFormat HH:mm:ss
    axisFormat %H:%M:%S
    test : milestone, m1, 00:03:38,0s
    section Bot 1
    Process Start : 00:00:00,0s
    Start﹕ Mine Stuff : 0s
    Walk to [10, 43] : 45s
    Mining rock-huge : 3s
    End : 3s
    Start﹕ Walk Around Stuff : 0s
    Walk to [10, 3] : 40s
    Walk to [10, 63] : 60s
    Walk to [0, 0] : 64s
    End : 3s
    section Bot 2
    Process Start : 00:00:00,0s
    Start﹕ Mine Stuff : 0s
    Walk to [20, 43] : 48s
    Mining rock-huge : 3s
    End : 0s
    Start﹕ Walk Around Stuff : 0s
    Walk to [20, 3] : 40s
    Walk to [20, 63] : 60s
    Walk to [0, 0] : 67s
    End : 0s
"#,
        );
    }
}
