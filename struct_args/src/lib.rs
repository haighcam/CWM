use anyhow::{Error, Result};

pub use struct_args_derive::*;

pub trait Arg: Sized {
    fn parse_args(args: &mut Vec<String>) -> Result<Self>;
    fn from_args() -> Result<Self> {
        let mut args = std::env::args().skip(1).rev().collect();
        Self::parse_args(&mut args)
    }
}

impl<E: std::error::Error + Sync + Send + 'static, T: std::str::FromStr<Err = E> + Sized> Arg
    for T
{
    fn parse_args(args: &mut Vec<String>) -> Result<T> {
        Ok(args
            .pop()
            .ok_or_else(|| Error::msg("No argument provided"))?
            .as_str()
            .parse()?)
    }
}


pub fn parse_u32(string: &str) -> Result<u32> {
    Ok(if let Some(string) = string.strip_prefix("0x") {
        u32::from_str_radix(string, 16)?
    } else if let Some(string) = string.strip_prefix('#') {
        u32::from_str_radix(string, 16)?
    } else {
        string.parse()?
    })
}
