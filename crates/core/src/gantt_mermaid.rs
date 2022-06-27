pub struct MermaidGanttBuilder {
    title: String,
    date_format: String,
    axis_format: String,
    buffer: String,
}

impl MermaidGanttBuilder {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            date_format: "HH:mm:ss".to_owned(),
            axis_format: "%H:%M:%S".to_owned(),
            buffer: String::new(),
        }
    }

    pub fn build(&self) -> String {
        let mut output = "gantt\n".to_owned();
        output += &format!("    title {}\n", self.title);
        output += &format!("    dateFormat {}\n", self.date_format);
        output += &format!("    axisFormat {}\n", self.axis_format);
        output += &self.buffer;
        output
    }

    #[allow(dead_code)]
    pub fn date_format(mut self, date_format: &str) -> Self {
        self.date_format = date_format.to_owned();

        self
    }

    #[allow(dead_code)]
    pub fn axis_format(mut self, axis_format: &str) -> Self {
        self.axis_format = axis_format.to_owned();

        self
    }

    pub fn add_milestone(
        mut self,
        label: &str,
        name: &str,
        timestamp: &str,
        duration: f64,
    ) -> Self {
        let label = self.replace_colon(label);
        let name = self.replace_colon(name);
        let line = format!("    {label} : milestone, {name}, {timestamp},{duration}s\n");
        self.buffer += &line;

        self
    }

    pub fn add_section(mut self, label: &str) -> Self {
        let label = self.replace_colon(label);
        let line = format!("    section {label}\n");
        self.buffer += &line;

        self
    }

    fn replace_colon(&self, input: &str) -> String {
        input.to_owned().replace(':', "﹕")
    }

    pub fn add_action(mut self, label: &str, duration: f64, timestamp: Option<&str>) -> Self {
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

#[cfg(test)]
mod tests {
    use crate::graph::task_graph::PositionRadius;
    use crate::plan::plan_builder::PlanBuilder;
    use crate::plan::planner::Planner;
    use crate::test_utils::fixture_world;
    use crate::types::Position;
    use std::sync::Arc;

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
        let graph = planner.graph();
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
            graph.mermaid_gantt(all_bots, "Example diagram"),
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
