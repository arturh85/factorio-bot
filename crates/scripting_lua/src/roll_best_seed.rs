use crate::lua_runner::run_lua;
use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use factorio_bot_core::paris::{error, info};
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::process::instance_setup::setup_factorio_instance;
use factorio_bot_core::settings::FactorioSettings;
use factorio_bot_core::tokio::sync::Mutex;
use factorio_bot_core::types::{AreaFilter, FactorioEntity, PlayerId, Position};
use std::cmp::Ordering;
use std::fs::read_to_string;
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

#[derive(Debug, Copy, Clone)]
pub enum RollSeedLimit {
    Rolls(u64),
    Seconds(u64),
}
#[allow(unused_mut, unused_variables)]
pub async fn roll_seed(
    settings: FactorioSettings,
    map_exchange_string: String,
    limit: RollSeedLimit,
    parallel: u8,
    plan_name: String,
    bot_count: PlayerId,
) -> Result<Option<(u32, f64)>> {
    let roll: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
    let best_seed_with_score: Arc<Mutex<Option<(u32, f64)>>> = Arc::new(Mutex::new(None));
    let workspace_path: Arc<String> = Arc::new(settings.workspace_path.to_string());
    let factorio_archive_path: Arc<String> = Arc::new(settings.factorio_archive_path.to_string());
    let map_exchange_string = Arc::new(map_exchange_string);

    let mut join_handles: Vec<JoinHandle<()>> = vec![];
    info!("preparing instances ...");
    for p in 0..parallel {
        let instance_name = format!("roll{}", p + 1);
        let rcon_settings = RconSettings {
            host: None,
            pass: "roll".into(),
            port: 1234 + p as u16,
        };
        let factorio_port: u16 = 2345 + p as u16;
        setup_factorio_instance(
            &workspace_path,
            &factorio_archive_path,
            &rcon_settings,
            Some(factorio_port),
            &instance_name,
            true,
            true,
            Some(map_exchange_string.to_string()),
            None,
            true,
        )
        .await
        .expect("failed to initially setup instance");
    }
    info!("finished preparing. spawning {} instances", parallel);
    let started = Instant::now();
    for p in 0..parallel {
        let instance_name = format!("roll{}", p + 1);
        let rcon_settings = RconSettings {
            host: None,
            pass: "roll".into(),
            port: 1234 + p as u16,
        };
        let factorio_port: u16 = 2345 + p as u16;
        let best_seed_with_score = best_seed_with_score.clone();
        let workspace_path = workspace_path.clone();
        let factorio_archive_path = factorio_archive_path.clone();
        let map_exchange_string = map_exchange_string.clone();
        let plan_name = plan_name.clone();
        let roll = roll.clone();
        let lua_path_str = format!("plans/{}.lua", plan_name);
        let lua_path = Path::new(&lua_path_str);
        let lua_path = std::fs::canonicalize(lua_path).into_diagnostic()?;
        if !lua_path.exists() {
            panic!("plan {} not found at {}", plan_name, lua_path_str);
        }
        let lua_code = read_to_string(lua_path).into_diagnostic()?;
        // join_handles.push(std::thread::spawn(move || {
        //     task::spawn(async move {
        //         while match limit {
        //             RollSeedLimit::Rolls(max_rolls) => *roll.lock().await < max_rolls,
        //             RollSeedLimit::Seconds(max_seconds) => {
        //                 started.elapsed() < std::time::Duration::from_secs(max_seconds)
        //             }
        //         } {
        //             let roll_started = Instant::now();
        //             let mut roll_mutex = roll.lock().await;
        //             *roll_mutex += 1;
        //             let roll: u64 = *roll_mutex;
        //             drop(roll_mutex);
        //
        //             let seed: u32 = rand::random();
        //             let instance = FactorioInstance::start(
        //                 &settings,
        //                 None,
        //                 0,
        //                 true,
        //                 Some(&instance_name),
        //                 Some(factorio_port),
        //                 Some(map_exchange_string.to_owned()),
        //                 Some(seed.to_owned()),
        //                 true,
        //                 true,
        //                 FactorioStartCondition::DiscoveryComplete
        //             )
        //             .await
        //             .expect("failed to start instance");
        //             // info!(
        //             //     "generated {} in <yellow>{:?}</>",
        //             //     seed,
        //             //     roll_started.elapsed()
        //             // );
        //             match score_seed(instance.rcon.clone(), instance.world.as_ref().unwrap().clone(), seed, lua_code.to_owned(), bot_count)
        //                 .await {
        //                 Ok(score) => {
        //                     let mut best_seed_with_score = best_seed_with_score.lock().await;
        //                     if let Some((_, previous_score)) = *best_seed_with_score {
        //                         if score > previous_score {
        //                             (*best_seed_with_score) = Some((seed, score));
        //                         }
        //                     } else {
        //                         (*best_seed_with_score) = Some((seed, score));
        //                     }
        //                     info!(
        //                         "instance #{} rolled #{}: seed {}{}</> scored {}{}</> in <yellow>{:?}</>",
        //                         p + 1,
        //                         roll,
        //                         if score > -10000. { "<bold><blue>" } else { "" },
        //                         seed,
        //                         if score > -10000. { "<bold><green>" } else { "" },
        //                         score,
        //                         roll_started.elapsed()
        //                     );
        //                 },
        //                 Err(err) => {
        //                     warn!("instance #{} rolled #{} with seed {} but failed: {:?}", p + 1,
        //                           roll,seed, err);
        //                 }
        //             }
        //             instance.stop().expect("failed to kill child");
        //         }
        //     });
        // }));
    }
    for join_handle in join_handles {
        join_handle.join().unwrap();
    }

    info!(
        "scored <green>{}</> seeds in <yellow>{:?}</>",
        *roll.lock().await,
        started.elapsed()
    );
    let best_seed_with_score = best_seed_with_score.lock().await;
    match *best_seed_with_score {
        Some((best_seed, best_score)) => {
            info!("best <blue>{}</> with score {}", best_seed, best_score)
        }
        None => error!("no best? {:?}", limit),
    }
    Ok(*best_seed_with_score)
}

pub async fn score_seed(
    rcon: Arc<FactorioRcon>,
    world: Arc<FactorioWorld>,
    _seed: u32,
    lua_code: String,
    bot_count: PlayerId,
) -> Result<f64> {
    let _rcon = rcon.clone();
    let mut planner = Planner::new(world, Some(_rcon.clone()));
    run_lua(&mut planner, &lua_code, None, bot_count, false).await?;
    let mut score = 0.0;

    let weight = planner.graph().shortest_path().expect("no path found");
    score -= weight;
    let center = Position::new(0., 0.);
    let resources = vec![
        "rock-huge",
        "iron-ore",
        "coal",
        "copper-ore",
        "stone",
        "crude-oil",
    ];
    for resource in resources {
        let nearest =
            find_nearest_entities(rcon.clone(), &center, 3000., Some(resource.into()), None)
                .await?;
        match nearest.is_empty() {
            false => {
                // info!("nearest {} @ {}/{}", resource, nearest.x(), nearest.y());
                // score -= calculate_distance(&center, &nearest[0].position);
            }
            true => {
                // warn!("not found: {}", resource);
                score -= 10000.;
            }
        }
    }
    // info!("scored {} in <yellow>{:?}</>", seed, started.elapsed());
    Ok(score.floor())
}

pub async fn find_nearest_entities(
    rcon: Arc<FactorioRcon>,
    search_center: &Position,
    search_radius: f64,
    name: Option<String>,
    entity_type: Option<String>,
) -> Result<Vec<FactorioEntity>> {
    let mut entities = rcon
        .find_entities_filtered(
            &AreaFilter::PositionRadius((search_center.clone(), Some(search_radius))),
            name,
            entity_type,
        )
        .await?;
    entities.sort_by(|a, b| {
        let da = calculate_distance(&a.position, search_center);
        let db = calculate_distance(&b.position, search_center);
        if da < db {
            Ordering::Less
        } else if da > db {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    });
    Ok(entities)
}
