use simplelog::*;
use std::fs::File;
use std::time::SystemTime;

fn main() {
    let time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Info,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            Config::default(),
            File::create(format!("/tmp/cwm-{:X}.log", time)).unwrap(),
        ),
    ])
    .unwrap();

    //let wm = x11rb::connect(None).map(|(dpy, screen)| WindowManager::new(dpy, screen)).unwrap();
    cwm::run_wm();
    print!("Done");
}
