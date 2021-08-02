use simplelog::*;

fn main() {
    SimpleLogger::init(LevelFilter::Info, Config::default()).unwrap();
    
    //let wm = x11rb::connect(None).map(|(dpy, screen)| WindowManager::new(dpy, screen)).unwrap();
    cwm::window_manager::run_wm();
    print!("Done");
}