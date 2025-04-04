use std::{env, io, path::PathBuf, str::FromStr};

use anyhow::Result;
use clap::{
    Arg, ArgAction, Command, ValueHint, crate_name, crate_version, value_parser,
};
use clap_complete::{Generator, Shell, generate};
use lazybar_core::{PROJ_DIRS, parser};
use log::LevelFilter;
use simple_logger::SimpleLogger;

fn print_completions<G: Generator>(r#gen: G, cmd: &mut Command) {
    generate(r#gen, cmd, cmd.get_name().to_string(), &mut io::stdout());
}

fn main() -> Result<()> {
    let mut cmd = Command::new(crate_name!())
        .version(crate_version!())
        .arg(
            Arg::new("generate")
                .short('g')
                .long("generate")
                .visible_aliases(["shell", "completion"])
                .help("Generates shell completions")
                .long_help("Generates completions for a given shell.")
                .value_name("SHELL")
                .value_hint(ValueHint::Other)
                .value_parser(value_parser!(Shell))
                .action(ArgAction::Set)
                .exclusive(true),
        )
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .help("Sets the config path")
                .long_help(
                    "Sets the config path\nIf unset, tries to find \
                     $XDG_CONFIG_HOME/lazybar/config.toml, \
                     $HOME/.config/lazybar/config.toml, and \
                     /etc/lazybar/config.toml",
                )
                .value_name("FILE")
                .value_hint(ValueHint::FilePath)
                .value_parser(value_parser!(PathBuf))
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("bar")
                .help(
                    "Specifies the name of the bar to read from the config \
                     file",
                )
                .value_name("BAR")
                .value_hint(ValueHint::Other)
                .action(ArgAction::Set)
                .required(true),
        )
        .arg(
            Arg::new("verbosity")
                .short('v')
                .help("Increases the logging level up to three times")
                .action(ArgAction::Count),
        );
    let args = cmd.clone().get_matches();

    if let Some(&shell) = args.get_one::<Shell>("generate") {
        eprintln!("Generating completions for {shell:?}");
        print_completions(shell, &mut cmd);
        std::process::exit(0);
    }

    let level = match args.get_one::<u8>("verbosity") {
        None | Some(0) => LevelFilter::Warn,
        Some(1) => LevelFilter::Info,
        Some(2) => LevelFilter::Debug,
        Some(_) => LevelFilter::Trace,
    };

    SimpleLogger::new()
        .with_level(level)
        .env()
        .with_utc_timestamps()
        .init()
        .unwrap();

    let path = args.get_one::<PathBuf>("config").map_or_else(
        || {
            (*PROJ_DIRS).as_ref().map_or_else(
                || PathBuf::from_str("/etc/lazybar/config.toml").unwrap(),
                |p| {
                    let mut path = p.config_dir().to_path_buf();
                    path.push("config.toml");
                    path
                },
            )
        },
        PathBuf::clone,
    );

    let config = parser::parse(
        args.get_one::<String>("bar").unwrap().as_str(),
        path.as_path(),
    )?;

    config.run()?;

    Ok(())
}
