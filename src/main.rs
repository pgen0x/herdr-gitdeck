//! herdr-gitdeck: review, stage and commit changes across every repo under a folder,
//! and send line comments to the agent working on them.
//!
//! `herdr-gitdeck [DIR]`        run the TUI (DIR defaults to the cwd)
//! `herdr-gitdeck --action ID`  plugin action: toggle | open | close

mod actions;
mod app;
mod diff;
mod git;
mod herdr;
mod highlight;
mod repos;
mod scan;
mod ui;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--action") {
        return actions::run(args.get(1).context("--action needs an id")?);
    }
    let root = match args.first() {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir()?,
    };
    let root = root
        .canonicalize()
        .with_context(|| format!("no such folder: {}", root.display()))?;

    highlight::warm_up();
    let paths = repos::discover(&root);
    let scanner = scan::Scanner::start(paths.clone());
    let mut app = app::App::new(root, paths, scanner);

    // `init` switches to the alternate screen and restores it on panic too.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut app::App) -> Result<()> {
    // GITDECK_LOG=<file> appends per-frame timings, for chasing lag.
    let mut log = std::env::var("GITDECK_LOG").ok().and_then(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    });
    while !app.quit {
        let t = Instant::now();
        terminal.draw(|f| ui::draw(f, app))?;
        let drawn = t.elapsed();

        // Wait for input, then drain everything already queued: a burst (key repeat,
        // mouse wheel) costs one frame instead of one frame per event.
        let (mut keys, mut busy) = (0, Duration::ZERO);
        if event::poll(Duration::from_millis(150))? {
            loop {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        let k = Instant::now();
                        app.on_key(key);
                        busy += k.elapsed();
                        keys += 1;
                    }
                }
                if app.quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
        let k = Instant::now();
        let landed = app.tick();
        busy += k.elapsed();
        if let Some(f) = log.as_mut().filter(|_| keys + landed > 0) {
            use std::io::Write;
            let _ = writeln!(
                f,
                "draw {drawn:?} keys {keys} background {landed} busy {busy:?}"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod perf {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Text screenshot for the README:
    /// `GITDECK_ROOT=<dir> cargo test preview -- --ignored --nocapture`
    /// (`GITDECK_KEYS` = keys to press first, default "\r\r" = open the first diff).
    #[test]
    #[ignore]
    fn preview() {
        let root = PathBuf::from(std::env::var("GITDECK_ROOT").unwrap());
        let keys = std::env::var("GITDECK_KEYS").unwrap_or_else(|_| "\r\r".into());
        let paths = repos::discover(&root);
        let scanner = scan::Scanner::start(paths.clone());
        let mut app = app::App::new(root, paths, scanner);
        while app.tick() == 0 {
            std::thread::sleep(Duration::from_millis(20));
        }
        for c in keys.chars() {
            let code = if c == '\r' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            };
            app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        std::thread::sleep(Duration::from_millis(500)); // let highlighting land
        app.tick();
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 24)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer();
        for y in 0..buf.area.height {
            let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            println!("{}", row.trim_end());
        }
    }

    /// `cargo test --release perf -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn key_and_draw_timing() {
        let root = PathBuf::from(std::env::var("GITDECK_ROOT").unwrap());
        let paths = repos::discover(&root);
        let scanner = scan::Scanner::start(paths.clone());
        let mut app = app::App::new(root, paths, scanner);
        while app.tick() == 0 {
            std::thread::sleep(Duration::from_millis(20)); // wait for the first scan
        }
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 50)).unwrap();
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        for (label, code, n) in [
            ("repo j", KeyCode::Char('j'), 10),
            ("focus", KeyCode::Tab, 2),
            ("file j", KeyCode::Char('j'), 10),
            ("focus", KeyCode::Tab, 1),
            ("diff j", KeyCode::Char('j'), 30),
        ] {
            for _ in 0..n {
                let t = std::time::Instant::now();
                app.on_key(key(code));
                let k = t.elapsed();
                term.draw(|f| ui::draw(f, &mut app)).unwrap();
                println!("{label}: key {:?} draw {:?}", k, t.elapsed() - k);
            }
        }
    }
}
