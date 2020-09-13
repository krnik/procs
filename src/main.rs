mod column;
mod columns;
mod config;
mod process;
mod style;
mod term_info;
mod util;
mod view;
mod watcher;

use crate::column::Column;
use crate::columns::*;
use crate::config::*;
use crate::util::{adjust, lap};
use crate::view::View;
use crate::watcher::Watcher;
use anyhow::{Context, Error};
use console::Term;
use std::cmp;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::time::Instant;
use structopt::{clap, StructOpt};
use unicode_width::UnicodeWidthStr;

// ---------------------------------------------------------------------------------------------------------------------
// Opt
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, StructOpt)]
#[structopt(long_version(option_env!("LONG_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))))]
#[structopt(setting(clap::AppSettings::ColoredHelp))]
#[structopt(setting(clap::AppSettings::DeriveDisplayOrder))]
pub struct Opt {
    /// Keywords for search
    #[structopt(name = "KEYWORD")]
    pub keyword: Vec<String>,

    /// AND  logic for multi-keyword
    #[structopt(
        short = "a",
        long = "and",
        conflicts_with_all(&["or", "nand", "nor"])
    )]
    pub and: bool,

    /// OR   logic for multi-keyword
    #[structopt(
        short = "o",
        long = "or",
        conflicts_with_all(&["and", "nand", "nor"])
    )]
    pub or: bool,

    /// NAND logic for multi-keyword
    #[structopt(
        short = "d",
        long = "nand",
        conflicts_with_all(&["and", "or", "nor"])
    )]
    pub nand: bool,

    /// NOR  logic for multi-keyword
    #[structopt(
        short = "r",
        long = "nor",
        conflicts_with_all(&["and", "or", "nand"])
    )]
    pub nor: bool,

    /// Show list of kind
    #[structopt(short = "l", long = "list")]
    pub list: bool,

    /// Tree view
    #[structopt(short = "t", long = "tree")]
    pub tree: bool,

    /// Watch mode with default interval (1s)
    #[structopt(short = "w", long = "watch")]
    pub watch: bool,

    /// Watch mode with custom interval
    #[structopt(short = "W", long = "watch-interval", value_name = "second")]
    pub watch_interval: Option<u64>,

    #[structopt(skip)]
    pub watch_mode: bool,

    /// Insert column to slot
    #[structopt(
        value_name = "kind",
        short = "i",
        long = "insert",
        multiple(true),
        number_of_values(1)
    )]
    pub insert: Vec<String>,

    /// Sort column by ascending
    #[structopt(
        value_name = "kind",
        long = "sorta",
        conflicts_with_all(&["sortd", "tree"])
    )]
    pub sorta: Option<String>,

    /// Sort column by descending
    #[structopt(
        value_name = "kind",
        long = "sortd",
        conflicts_with_all(&["sorta", "tree"])
    )]
    pub sortd: Option<String>,

    /// Color mode
    #[structopt(
        short = "c",
        long = "color",
        possible_value = "auto",
        possible_value = "always",
        possible_value = "disable"
    )]
    pub color: Option<String>,

    /// Pager mode
    #[structopt(
        short = "p",
        long = "pager",
        possible_value = "auto",
        possible_value = "always",
        possible_value = "disable"
    )]
    pub pager: Option<String>,

    /// Interval to calculate throughput
    #[structopt(long = "interval", default_value = "100", value_name = "millisec")]
    pub interval: u64,

    /// Generate configuration sample file
    #[structopt(long = "config")]
    pub config: bool,

    /// Show debug message
    #[structopt(long = "debug", hidden = true)]
    pub debug: bool,
}

// ---------------------------------------------------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------------------------------------------------

#[cfg_attr(tarpaulin, skip)]
fn get_config() -> Result<Config, Error> {
    use std::env::var;
    use std::path::Path;

    let xdg_env = var("XDG_CONFIG_HOME").ok();
    let home_env = var("HOME").ok();

    let cfg_path = xdg_env.as_ref()
        .or(home_env.as_ref())
        .map(|cfg_path| Path::new(cfg_path).join("procs").join("config.toml"))
        .or_else(|| {
            home_env
                .as_ref()
                .map(|cfg| Path::new(cfg).join(".procs.toml"))
        })
        .filter(|path| path.exists());

    let config: Config = if let Some(path) = cfg_path {
        let mut f = fs::File::open(&path).context(format!("failed to open file ({:?})", path))?;
        let mut s = String::new();
        f.read_to_string(&mut s)
            .context(format!("failed to read file ({:?})", path))?;
        toml::from_str(&s).context(format!("failed to parse toml ({:?})", path))?
    } else {
        toml::from_str(CONFIG_DEFAULT).unwrap()
    };

    Ok(config)
}

// ---------------------------------------------------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------------------------------------------------

#[cfg_attr(tarpaulin, skip)]
fn main() {
    let err = Term::stderr();

    if let Err(x) = run() {
        let mut cause = x.chain();
        let _ = err.write_line(&format!(
            "{} {}",
            console::style("error:").red().bold(),
            cause.next().unwrap()
        ));

        for x in cause {
            let _ = err.write_line(&format!("  {} {}", console::style("caused by:").red(), x));
        }

        std::process::exit(1);
    }
}

#[cfg_attr(tarpaulin, skip)]
fn run() -> Result<(), Error> {
    let mut opt = Opt::from_args();
    opt.watch_mode = opt.watch || opt.watch_interval.is_some();

    if opt.config {
        run_config()
    } else if opt.list {
        run_list()
    } else {
        let config = get_config()?;

        if opt.watch_mode {
            let interval = opt.watch_interval.unwrap_or(1);
            run_watch(&opt, &config, interval)
        } else {
            run_default(&opt, &config)
        }
    }
}

fn run_config() -> Result<(), Error> {
    let config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
    let toml = toml::to_string(&config)?;
    println!("{}", toml);
    Ok(())
}

fn run_list() -> Result<(), Error> {
    let mut width = 0;
    let mut list = Vec::new();
    let mut desc = HashMap::new();
    for (_, (v, d)) in KIND_LIST.iter() {
        list.push(v);
        desc.insert(v, d);
        width = cmp::max(width, UnicodeWidthStr::width(*v));
    }

    list.sort();

    println!("Column kind list:");
    for l in list {
        println!(
            "  {}: {}",
            adjust(l, width, &ConfigColumnAlign::Left),
            desc[l]
        );
    }

    Ok(())
}

#[cfg_attr(tarpaulin, skip)]
fn run_watch(opt: &Opt, config: &Config, interval: u64) -> Result<(), Error> {
    Watcher::start(opt, config, interval)
}

fn run_default(opt: &Opt, config: &Config) -> Result<(), Error> {
    let mut time = Instant::now();

    let mut view = View::new(opt, config, false);

    if opt.debug {
        lap(&mut time, "Info: View::new");
    }

    view.filter(opt, config);

    if opt.debug {
        lap(&mut time, "Info: view.filter");
    }

    view.adjust(config, &HashMap::new());

    if opt.debug {
        lap(&mut time, "Info: view.adjust");
    }

    view.display(opt, config)?;

    if opt.debug {
        lap(&mut time, "Info: view.display");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;

        let args = vec!["procs"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_search() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;

        let args = vec!["procs", "root"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());

        let args = vec!["procs", "1"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());

        let args = vec!["procs", "--or", "root", "1"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());

        let args = vec!["procs", "--and", "root", "1"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());

        let args = vec!["procs", "--nor", "root", "1"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());

        let args = vec!["procs", "--nand", "root", "1"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());

        config.search.nonnumeric_search = ConfigSearchKind::Exact;
        config.search.numeric_search = ConfigSearchKind::Partial;
        let args = vec!["procs", "root", "1"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_config() {
        let ret = run_config();
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_list() {
        let ret = run_list();
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_without_truncate() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.display.cut_to_terminal = false;

        let args = vec!["procs"];
        let opt = Opt::from_iter(args.iter());
        config.pager.mode = ConfigPagerMode::Disable;
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_insert() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;

        let args = vec!["procs", "--insert", "ppid"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_sort() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;

        let args = vec!["procs", "--sorta", "cpu"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());

        let args = vec!["procs", "--sortd", "cpu"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_tree() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;

        let args = vec!["procs", "--tree"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_all() {
        let mut config: Config = toml::from_str(CONFIG_ALL).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;

        let _tcp = std::net::TcpListener::bind("127.0.0.1:10000");
        let _udp = std::net::UdpSocket::bind("127.0.0.1:10000");

        let args = vec!["procs"];
        let opt = Opt::from_iter(args.iter());
        let ret = run_default(&opt, &config);
        assert!(ret.is_ok());
    }
}
