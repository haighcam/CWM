use anyhow::Result;
use std::env::{args, Args};
use struct_args::Arg;

#[derive(Arg, Debug)]
enum Test {
    #[struct_args_match(ND, "a", "b", "C")]
    A(usize),
    B {
        x: f32,
    },
}

/*
impl Arg for Test {
    fn from_args(args: &mut Args) -> Result<Self> {
        Ok(Self(usize::from_args(args)?))
    }
}
*/

fn main() -> Result<()> {
    let mut args = args();
    args.next();
    let test = Test::from_args(&mut args)?;
    println!("{:?}", test);
    Ok(())
}
