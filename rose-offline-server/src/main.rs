// We must globally allow dead_code because of modular-bitfield..
#![allow(dead_code)]
#![allow(clippy::enum_variant_names)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::needless_option_as_deref)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

mod game;
mod irose;
mod protocol;

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

use clap::{Arg, Command};
use log::debug;
use simplelog::*;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::Notify;

use rose_file_readers::{
    HostFilesystemDevice, VfsIndex, VirtualFilesystem, VirtualFilesystemDevice,
};

use crate::{
    game::{
        api::{start_api_server, ApiServerConfig, ApiState, LlmBotManager},
        GameConfig,
    },
    protocol::server::{GameServer, LoginServer, WorldServer},
};

pub enum ProtocolType {
    Irose,
}

impl Default for ProtocolType {
    fn default() -> Self {
        Self::Irose
    }
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    if let Err(e) = TermLogger::init(
        LevelFilter::Trace,
        ConfigBuilder::new()
            .set_location_level(LevelFilter::Trace)
            .add_filter_ignore_str("mio")
            .add_filter_ignore_str("npc_ai")
            .add_filter_ignore_str("packets")
            .add_filter_ignore_str("quest")
            .add_filter_ignore_str("tracing::span")
            .add_filter_ignore_str("big-brain")
            .build(),
        TerminalMode::Stdout,
        ColorChoice::Auto,
    ) {
        eprintln!("Failed to initialise logging: {}", e);
        return Err(e.into());
    }

    let mut command = Command::new("rose-offline")
        .arg(
            Arg::new("data-idx")
                .long("data-idx")
                .help("Path to data.idx")
                .takes_value(true),
        )
        .arg(
            Arg::new("data-path")
                .long("data-path")
                .help("Optional path to extracted data, any files here override ones in data.idx")
                .takes_value(true),
        )
        .arg(
            Arg::new("ip")
                .long("ip")
                .help("Listen IP used for login, world, game servers")
                .takes_value(true)
                .default_value("127.0.0.1"),
        )
        .arg(
            Arg::new("login-port")
                .long("login-port")
                .help("Port for login server")
                .takes_value(true)
                .default_value("29000"),
        )
        .arg(
            Arg::new("world-port")
                .long("world-port")
                .help("Port for world server")
                .takes_value(true)
                .default_value("29100"),
        )
        .arg(
            Arg::new("game-port")
                .long("game-port")
                .help("Port for login server")
                .takes_value(true)
                .default_value("29200"),
        )
        .arg(
            clap::Arg::new("protocol")
                .long("protocol")
                .takes_value(true)
                .value_parser(["irose"])
                .default_value("irose")
                .help("Select which protocol to use."),
        )
        .arg(
            Arg::new("api-port")
                .long("api-port")
                .help("Port for LLM Buddy Bot REST API server")
                .takes_value(true)
                .default_value("3000"),
        )
        .arg(
            Arg::new("enable-api")
                .long("enable-api")
                .help("Enable the LLM Buddy Bot REST API server")
                .takes_value(false),
        );
    let data_path_error = command.error(
        clap::ErrorKind::ArgumentNotFound,
        "Must specify at least one of --data-idx or --data-path",
    );
    let matches = command.get_matches();
    
    let listen_ip = match matches.value_of("ip") {
        Some(ip) => ip,
        None => {
            log::error!("Failed to get 'ip' argument, which has a default value");
            return Err("Failed to get 'ip' argument".into());
        }
    };
    
    let login_port = match matches.value_of("login-port") {
        Some(port) => port,
        None => {
            log::error!("Failed to get 'login-port' argument, which has a default value");
            return Err("Failed to get 'login-port' argument".into());
        }
    };
    
    let world_port = match matches.value_of("world-port") {
        Some(port) => port,
        None => {
            log::error!("Failed to get 'world-port' argument, which has a default value");
            return Err("Failed to get 'world-port' argument".into());
        }
    };
    
    let game_port = match matches.value_of("game-port") {
        Some(port) => port,
        None => {
            log::error!("Failed to get 'game-port' argument, which has a default value");
            return Err("Failed to get 'game-port' argument".into());
        }
    };
    
    let api_port: u16 = match matches.value_of("api-port") {
        Some(port) => port.parse().unwrap_or_else(|e| {
            log::error!("Failed to parse 'api-port' value: {}", e);
            8080
        }),
        None => {
            log::error!("Failed to get 'api-port' argument, which has a default value");
            return Err("Failed to get 'api-port' argument".into());
        }
    };
    
    let enable_api = matches.is_present("enable-api");
    let protocol_type = match matches.value_of("protocol") {
        Some("irose") => ProtocolType::Irose,
        _ => ProtocolType::default(),
    };

    let (login_protocol, world_protocol, game_protocol) = match protocol_type {
        ProtocolType::Irose => (
            irose::login_protocol(),
            irose::world_protocol(),
            irose::game_protocol(),
        ),
    };

    let mut data_idx_path = matches.value_of("data-idx").map(Path::new);
    let data_extracted_path = matches.value_of("data-path").map(Path::new);
    if data_idx_path.is_none() && data_extracted_path.is_none() {
        if Path::new("data.idx").exists() {
            data_idx_path = Some(Path::new("data.idx"));
        } else {
            data_path_error.exit();
        }
    }

    let mut vfs_devices: Vec<Box<dyn VirtualFilesystemDevice + Send + Sync>> = Vec::new();
    if let Some(data_extracted_path) = data_extracted_path {
        log::info!(
            "Loading game data from path {}",
            data_extracted_path.to_string_lossy()
        );
        vfs_devices.push(Box::new(HostFilesystemDevice::new(
            data_extracted_path.to_path_buf(),
        )));
    }

    if let Some(data_idx_path) = data_idx_path {
        log::info!(
            "Loading game data from vfs {}",
            data_idx_path.to_string_lossy()
        );
        let vfs_index = match VfsIndex::load(data_idx_path) {
            Ok(index) => index,
            Err(e) => {
                log::error!(
                    "Failed to load VFS index from {}: {}",
                    data_idx_path.to_string_lossy(),
                    e
                );
                return Err(e.into());
            }
        };
        vfs_devices.push(Box::new(vfs_index));

        let index_root_path = data_idx_path
            .parent()
            .map(|path| path.into())
            .unwrap_or_else(PathBuf::new);
        log::info!(
            "Loading game data from vfs root path {}",
            index_root_path.to_string_lossy()
        );
        vfs_devices.push(Box::new(HostFilesystemDevice::new(index_root_path)));
    }

    let virtual_filesystem = VirtualFilesystem::new(vfs_devices);

    let started_load = std::time::Instant::now();
    let game_data = irose::get_game_data(&virtual_filesystem);
    debug!("Time take to read game data {:?}", started_load.elapsed());

    let game_config = GameConfig::default();

    let (game_control_tx, game_control_rx) = crossbeam_channel::unbounded();

    // Create LLM Bot Manager if API is enabled
    // Note: We need to get the sender and bots map before moving the manager to the game thread
    let (llm_bot_manager, api_command_sender, shared_bots_map) = if enable_api {
        let manager = LlmBotManager::new();
        let sender = manager.command_sender();
        let bots_map = manager.bots_map();
        log::info!("LLM Buddy Bot API enabled on port {}", api_port);
        log::info!("Created LlmBotManager - sender and receiver are from the same unbounded channel");
        (Some(manager), Some(sender), Some(bots_map))
    } else {
        (None, None, None)
    };

    // Start game world thread
    let game_world_thread = {
        let game_config = game_config.clone();
        
        std::thread::spawn(move || {
            if let Some(manager) = llm_bot_manager {
                game::GameWorld::with_llm_bot_manager(game_control_rx, manager)
                    .run(game_config, game_data);
            } else {
                game::GameWorld::new(game_control_rx).run(game_config, game_data);
            }
        })
    };

    // Start API server if enabled
    if let (Some(sender), Some(bots_map)) = (api_command_sender, shared_bots_map) {
        // Create API state with the command sender and shared bots map
        let api_state = ApiState::new(sender, bots_map);
        let api_config = ApiServerConfig::new(api_port)
            .with_host(listen_ip.to_string());
        
        // Create shutdown signal
        let shutdown_signal = Arc::new(Notify::new());
        let shutdown_signal_clone = shutdown_signal.clone();

        // Spawn API server in tokio runtime
        tokio::spawn(async move {
            if let Err(e) = start_api_server(api_state, api_config, Some(shutdown_signal_clone)).await {
                log::error!("API server error: {}", e);
            }
        });

        log::info!("LLM Buddy Bot REST API server started on {}:{}", listen_ip, api_port);
    }

    let login_listener = match TcpListener::bind(format!("{}:{}", listen_ip, login_port)).await {
        Ok(listener) => listener,
        Err(e) => {
            log::error!(
                "Failed to bind login server to {}:{}: {}",
                listen_ip,
                login_port,
                e
            );
            return Err(e.into());
        }
    };

    let mut login_server = match LoginServer::new(login_listener, login_protocol, game_control_tx.clone()).await {
        Ok(server) => server,
        Err(e) => {
            log::error!("Failed to initialize login server: {}", e);
            return Err(e.into());
        }
    };

    let world_listener = match TcpListener::bind(format!("{}:{}", listen_ip, world_port)).await {
        Ok(listener) => listener,
        Err(e) => {
            log::error!(
                "Failed to bind world server to {}:{}: {}",
                listen_ip,
                world_port,
                e
            );
            return Err(e.into());
        }
    };

    let mut world_server = match WorldServer::new(
        String::from("_WorldServer"),
        world_listener,
        world_protocol,
        game_control_tx.clone(),
    )
    .await
    {
        Ok(server) => server,
        Err(e) => {
            log::error!("Failed to initialize world server: {}", e);
            return Err(e.into());
        }
    };

    let game_listener = match TcpListener::bind(format!("{}:{}", listen_ip, game_port)).await {
        Ok(listener) => listener,
        Err(e) => {
            log::error!(
                "Failed to bind game server to {}:{}: {}",
                listen_ip,
                game_port,
                e
            );
            return Err(e.into());
        }
    };

    let mut game_server = match GameServer::new(
        String::from("GameServer"),
        world_server.get_entity(),
        game_listener,
        game_protocol,
        game_control_tx.clone(),
    )
    .await
    {
        Ok(server) => server,
        Err(e) => {
            log::error!("Failed to initialize game server: {}", e);
            return Err(e.into());
        }
    };

    tokio::spawn(async move {
        game_server.run().await;
    });

    tokio::spawn(async move {
        world_server.run().await;
    });

    login_server.run().await;

    // Wait for game world thread to finish
    let _ = game_world_thread.join();
    
    Ok(())
}

fn main() -> ExitCode {
    let rt = match Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("Failed to create tokio runtime: {}", e);
            return ExitCode::from(1);
        }
    };

    if let Err(e) = rt.block_on(async {
        async_main().await
    }) {
        log::error!("Server initialization failed: {}", e);
        return ExitCode::from(1);
    }
    
    ExitCode::SUCCESS
}
