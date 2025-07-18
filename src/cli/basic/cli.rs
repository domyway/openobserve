// Copyright 2025 OpenObserve Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use chrono::TimeZone;
use config::utils::file::set_permission;
use infra::{
    db::{ORM_CLIENT, connect_to_orm},
    file_list as infra_file_list, table,
};

use crate::{
    cli::data::{
        Context,
        cli::{Cli as dataCli, args as dataArgs},
        export, import,
    },
    common::{infra::config::USERS, meta},
    migration,
    service::{compact, db, file_list, users},
};

pub async fn cli() -> Result<bool, anyhow::Error> {
    let app = clap::Command::new("openobserve")
        .version(config::VERSION)
        .about(clap::crate_description!())
        .subcommands(&[
            clap::Command::new("reset")
                .about("reset openobserve data")
                .arg(
                    clap::Arg::new("component")
                        .short('c')
                        .long("component")
                        .help(
                            "reset data of the component: root, user, alert, dashboard, function, stream-stats",
                        ),
                ),
            clap::Command::new("import")
                .about("import openobserve data").args(dataArgs()),
            clap::Command::new("export")
                .about("export openobserve data").args(dataArgs()),
            clap::Command::new("view")
                .about("view openobserve data")
                .arg(
                    clap::Arg::new("component")
                        .short('c')
                        .long("component")
                        .help("view data of the component: version, user"),
                ),
            clap::Command::new("init-dir")
                .about("init openobserve data dir")
                .arg(
                    clap::Arg::new("path")
                        .short('p')
                        .long("path")
                        .help("init this path as data root dir"),
                ),
            clap::Command::new("migrate-file-list")
                .about("migrate file-list")
                .args([
                    clap::Arg::new("prefix")
                        .short('p')
                        .long("prefix")
                        .value_name("prefix")
                        .required(false)
                        .help("only migrate specified prefix, default is all"),
                    clap::Arg::new("from")
                        .short('f')
                        .long("from")
                        .value_name("from")
                        .required(true)
                        .help("migrate from: sled, sqlite, etcd, mysql, postgresql"),
                    clap::Arg::new("to")
                        .short('t')
                        .long("to")
                        .value_name("to")
                        .required(true)
                        .help("migrate to: sqlite, mysql, postgresql"),
                ]),
            clap::Command::new("migrate-meta")
                .about("migrate meta")
                .args([
                    clap::Arg::new("from")
                        .short('f')
                        .long("from")
                        .value_name("from")
                        .required(true)
                        .help("migrate from: sled, sqlite, etcd, mysql, postgresql"),
                    clap::Arg::new("to")
                        .short('t')
                        .long("to")
                        .value_name("to")
                        .required(true)
                        .help("migrate to: sqlite, etcd, mysql, postgresql"),
                ]),
            clap::Command::new("migrate-dashboards").about("migrate-dashboards"),
            clap::Command::new("migrate-pipeline").about("migrate pipelines")
                .arg(
                    clap::Arg::new("drop-table")
                        .long("drop-table")
                        .required(false)
                        .num_args(0)
                        .help("Drop existing Pipeline table first before migrating")
                ),
            clap::Command::new("delete-parquet")
                .about("delete parquet files from s3 and file_list")
                .args([
                    clap::Arg::new("account")
                        .short('a')
                        .long("account")
                        .required(false)
                        .value_name("account")
                        .help("the account name"),
                    clap::Arg::new("file")
                        .short('f')
                        .long("file")
                        .value_name("file")
                        .help("the parquet file name"),
                ]),
            clap::Command::new("migrate-schemas").about("migrate from single row to row per schema version"),
            clap::Command::new("seaorm-rollback").about("rollback SeaORM migration steps")
                .subcommand(
                    clap::Command::new("all")
                    .about("rollback all SeaORM migration steps")
                )
                .subcommand(
                    clap::Command::new("last")
                    .about("rollback last N SeaORM migration steps")
                    .arg(clap::Arg::new("N").help("number of migration steps to rollback (default is 1)").value_parser(clap::value_parser!(u32)))
                ),
            clap::Command::new("recover-file-list").about("recover file list from s3")
                .args([
                    clap::Arg::new("account")
                        .short('a')
                        .long("account")
                        .value_name("account")
                        .required(true)
                        .help("the account name"),
                    clap::Arg::new("prefix")
                        .short('p')
                        .long("prefix")
                        .value_name("prefix")
                        .required(true)
                        .help("only migrate specified prefix"),
                    clap::Arg::new("insert")
                        .short('i')
                        .long("insert")
                        .value_name("insert")
                        .required(false)
                        .action(clap::ArgAction::SetTrue)
                        .help("insert file list into db"),
                ]),
            clap::Command::new("node").about("node command").subcommands([
                clap::Command::new("offline").about("offline node"),
                clap::Command::new("online").about("online node"),
                clap::Command::new("flush").about("flush memtable to disk"),
                clap::Command::new("list").about("list cached nodes").args([
                    clap::Arg::new("metrics")
                        .short('m')
                        .long("metrics")
                        .required(false)
                        .action(clap::ArgAction::SetTrue)
                        .help("show node metrics"),
                ]),
                clap::Command::new("metrics").about("show local node metrics"),
            ]),
            clap::Command::new("sql").about("query data").args([
                clap::Arg::new("org") 
                    .long("org")
                    .required(false)
                    .default_value("default")
                    .help("org name"),
                clap::Arg::new("execute")
                    .short('e')
                    .long("execute")
                    .required(true)
                    .help("execute sql"),
                clap::Arg::new("time")
                    .short('t')
                    .long("time")
                    .required(false)
                    .default_value("15m")
                    .help("time range, e.g. 15m, 1h, 1d, 1w, 1y"),
                clap::Arg::new("limit")
                    .short('l')
                    .long("limit")
                    .required(false)
                    .default_value("10")
                    .help("limit the number of results"),
            ]),
            clap::Command::new("test").about("test command").subcommands([
                clap::Command::new("file_list").about("test generate file list groups").args([
                    clap::Arg::new("mode")
                        .short('m')
                        .long("mode")
                        .required(true)
                        .default_value("file_size")
                        .help("mode: file_size, file_time, time_range"),
                    clap::Arg::new("stream")
                        .short('s')
                        .long("stream")
                        .required(true)
                        .default_value("")
                        .help("stream name, the format is org/logs/default"),
                    clap::Arg::new("hour")
                        .short('d')
                        .long("date")
                        .required(true)
                        .default_value("")
                        .help("date for testing, the format is 2025/01/01/00"),
                    clap::Arg::new("group_size")
                        .short('g')
                        .long("group_size")
                        .required(false)
                        .default_value("5")
                        .help("group size by gb, default is 5gb"),
                ]),
            ]),
            clap::Command::new("parse-id").about("parse snowflake id to timestamp").args([
                clap::Arg::new("id")
                    .short('i')
                    .long("id")
                    .required(true)
                    .help("snowflake id"),
            ]),
            clap::Command::new("consistent-hash").about("consistent hash").args([
                clap::Arg::new("file")
                    .short('f')
                    .long("file")
                    .required(true)
                    .num_args(1..)
                    .help("file"),
            ]),
            clap::Command::new("upgrade-db")
                .about("upgrade db table schemas").args(dataArgs()),
        ])
        .get_matches();

    if app.subcommand().is_none() {
        return Ok(false);
    }

    #[cfg(not(feature = "tokio-console"))]
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("INFO"));

    let (name, command) = app.subcommand().unwrap();
    if name == "init-dir" {
        match command.get_one::<String>("path") {
            Some(path) => {
                set_permission(path, 0o777)?;
                println!("init dir {path} successfully");
            }
            None => {
                return Err(anyhow::anyhow!("please set data path"));
            }
        }
        return Ok(true);
    }

    // init infra, create data dir & tables
    let cfg = config::get_config();
    infra::init().await.expect("infra init failed");
    match name {
        "reset" => {
            let component = command.get_one::<String>("component").unwrap();
            match component.as_str() {
                "root" => {
                    let _ = users::update_user(
                        meta::organization::DEFAULT_ORG,
                        cfg.auth.root_user_email.as_str(),
                        false,
                        cfg.auth.root_user_email.as_str(),
                        meta::user::UpdateUser {
                            change_password: true,
                            old_password: None,
                            new_password: Some(cfg.auth.root_user_password.clone()),
                            role: Some(crate::common::meta::user::UserRoleRequest {
                                role: config::meta::user::UserRole::Root.to_string(),
                                custom: None,
                            }),
                            first_name: Some("root".to_owned()),
                            last_name: Some("".to_owned()),
                            token: if cfg.auth.root_user_token.is_empty() {
                                None
                            } else {
                                Some(cfg.auth.root_user_token.clone())
                            },
                        },
                    )
                    .await?;
                }
                "user" => {
                    db::user::reset().await?;
                }
                "alert" => {
                    db::alerts::alert::reset().await?;
                }
                "dashboard" => {
                    table::dashboards::delete_all().await?;
                }
                "report" => {
                    let conn = ORM_CLIENT.get_or_init(connect_to_orm).await;
                    db::dashboards::reports::reset(conn).await?;
                }
                "function" => {
                    db::functions::reset().await?;
                }
                "stream-stats" => {
                    // init nats client
                    let (tx, _rx) = tokio::sync::mpsc::channel::<infra::db::nats::NatsEvent>(1);
                    if !cfg.common.local_mode
                        && cfg.common.cluster_coordinator.to_lowercase() == "nats"
                    {
                        infra::db::nats::init_nats_client(tx).await?;
                    }
                    // reset stream stats update offset
                    db::compact::stats::set_offset(0, None).await?;
                    // reset stream stats table data
                    infra_file_list::reset_stream_stats().await?;
                    // load stream list
                    db::schema::cache().await?;
                    // update stats from file list
                    compact::stats::update_stats_from_file_list()
                        .await
                        .expect("file list remote calculate stats failed");
                }
                _ => {
                    return Err(anyhow::anyhow!(
                        "unsupported reset component: {}",
                        component
                    ));
                }
            }
        }
        "view" => {
            let component = command.get_one::<String>("component").unwrap();
            match component.as_str() {
                "version" => {
                    println!("version: {}", db::metas::version::get().await?);
                }
                "user" => {
                    db::user::cache().await?;
                    db::org_users::cache().await?;
                    let mut id = 0;
                    for user in USERS.iter() {
                        id += 1;
                        println!("{id}\t{:?}\n{:?}", user.key(), user.value());
                    }
                }
                _ => {
                    return Err(anyhow::anyhow!("unsupported reset component: {component}"));
                }
            }
        }
        "migrate-file-list" => {
            let from = match command.get_one::<String>("from") {
                Some(from) => from.to_string(),
                None => "".to_string(),
            };
            let to = match command.get_one::<String>("to") {
                Some(to) => to.to_string(),
                None => "".to_string(),
            };
            println!("Running migration file_list from {from} to {to}");
            migration::file_list::run(&from, &to).await?;
        }
        "migrate-meta" => {
            let from = match command.get_one::<String>("from") {
                Some(from) => from.to_string(),
                None => "".to_string(),
            };
            let to = match command.get_one::<String>("to") {
                Some(to) => to.to_string(),
                None => "".to_string(),
            };
            println!("Running migration metadata from {from} to {to}");
            migration::meta::run(&from, &to).await?
        }
        "migrate-dashboards" => {
            println!("Running migration dashboard");
            migration::dashboards::run().await?
        }
        "migrate-pipeline" => {
            println!("Running migration pipeline");
            let drop_table = command.get_flag("drop-table");
            migration::pipeline_func::run(drop_table).await?;
        }
        "delete-parquet" => {
            let account = command
                .get_one::<String>("account")
                .map(|s| s.to_string())
                .unwrap_or_default();
            let file = command.get_one::<String>("file").unwrap();
            match file_list::delete_parquet_file(&account, file, true).await {
                Ok(_) => {
                    println!("delete parquet file {file} successfully");
                }
                Err(e) => {
                    println!("delete parquet file {file} failed, error: {e}");
                }
            }
        }
        "import" => {
            crate::common::infra::init().await?;
            crate::common::infra::cluster::register_and_keep_alive().await?;
            import::Import::operator(dataCli::arg_matches(command.clone())).await?;
        }
        "export" => {
            crate::common::infra::init().await?;
            crate::common::infra::cluster::register_and_keep_alive().await?;
            export::Export::operator(dataCli::arg_matches(command.clone())).await?;
        }
        "migrate-schemas" => {
            println!("Running schema migration to row per schema version");
            #[allow(deprecated)]
            migration::schema::run().await?
        }
        "seaorm-rollback" => match command.subcommand() {
            Some(("all", _)) => {
                println!("Rolling back all");
                infra::table::down(None).await?
            }
            Some(("last", sub_matches)) => {
                let n = sub_matches
                    .get_one::<u32>("N")
                    .map(|n| n.to_owned())
                    .unwrap_or(1);
                println!("Rolling back {n}");
                infra::table::down(Some(n)).await?
            }
            Some((name, _)) => {
                return Err(anyhow::anyhow!("unsupported sub command: {name}"));
            }
            None => {
                return Err(anyhow::anyhow!("missing sub command"));
            }
        },
        "recover-file-list" => {
            let account = command
                .get_one::<String>("account")
                .map(|s| s.to_string())
                .unwrap_or_default();
            let prefix = command.get_one::<String>("prefix").unwrap();
            let insert = command.get_flag("insert");
            super::load::load_file_list_from_s3(&account, prefix, insert).await?;
        }
        "node" => {
            let command = command.subcommand();
            match command {
                Some(("offline", _)) => {
                    super::http::node_offline().await?;
                }
                Some(("online", _)) => {
                    super::http::node_online().await?;
                }
                Some(("flush", _)) => {
                    super::http::node_flush().await?;
                }
                Some(("list", args)) => {
                    let metrics = args.get_flag("metrics");
                    if metrics {
                        super::http::node_list_with_metrics().await?;
                    } else {
                        super::http::node_list().await?;
                    }
                }
                Some(("metrics", _)) => {
                    super::http::local_node_metrics().await?;
                }
                _ => {
                    return Err(anyhow::anyhow!("unsupported sub command: {name}"));
                }
            }
        }
        "sql" => {
            let org = command.get_one::<String>("org").unwrap();
            let sql = command.get_one::<String>("execute").unwrap();
            let time = command.get_one::<String>("time").unwrap();
            let limit = command.get_one::<String>("limit").unwrap();
            let mut limit = limit.parse::<i64>().unwrap_or(10);
            if !(1..=1000).contains(&limit) {
                limit = 10;
            }
            super::http::query(org, sql, time, limit).await?;
        }
        "test" => {
            let command = command.subcommand();
            match command {
                Some(("file_list", args)) => {
                    let mode = args.get_one::<String>("mode").unwrap();
                    let stream = args.get_one::<String>("stream").unwrap();
                    let hour = args.get_one::<String>("hour").unwrap();
                    let group_size = args.get_one::<String>("group_size").unwrap();
                    super::test::file_list(mode, stream, hour, group_size).await?;
                }
                _ => {
                    return Err(anyhow::anyhow!("unsupported sub command: {name}"));
                }
            }
        }
        "parse-id" => {
            let id = command.get_one::<String>("id").unwrap();
            println!("id: {id}");
            let id = id.parse::<i64>().unwrap();
            let ts = config::ider::to_timestamp_millis(id);
            println!("timestamp: {ts}");
            let t = chrono::Utc.timestamp_nanos(ts * 1_000_000);
            let td = t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
            println!("datetimes: {td}");
        }
        "consistent-hash" => {
            let files = command
                .get_many::<String>("file")
                .unwrap()
                .collect::<Vec<_>>();
            let files = files.iter().map(|f| f.to_string()).collect::<Vec<_>>();
            super::http::consistent_hash(files).await?;
        }
        "upgrade-db" => {
            crate::migration::init_db().await?;
        }
        _ => {
            return Err(anyhow::anyhow!("unsupported sub command: {name}"));
        }
    }

    // flush db
    let db = infra::db::get_db().await;
    if let Err(e) = db.close().await {
        log::error!("waiting for db close failed, error: {e}");
    }

    println!("command {name} execute successfully");
    Ok(true)
}
