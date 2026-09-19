use std::sync::atomic::Ordering;
use std::sync::Arc;

use mlua::Lua;

use super::execution;
use super::types::RuntimeControl;

pub fn register(lua: &Lua, control: Arc<RuntimeControl>) -> mlua::Result<()> {
    let module = lua.create_table()?;
    let paused = Arc::clone(&control);
    module.set(
        "is_paused",
        lua.create_function(move |_, ()| Ok(paused.paused.load(Ordering::Relaxed)))?,
    )?;
    let pause = Arc::clone(&control);
    module.set(
        "pause",
        lua.create_function(move |_, ()| {
            pause.paused.store(true, Ordering::Relaxed);
            Ok(())
        })?,
    )?;
    let resume = Arc::clone(&control);
    module.set(
        "resume",
        lua.create_function(move |_, ()| {
            resume.paused.store(false, Ordering::Relaxed);
            Ok(())
        })?,
    )?;
    module.set(
        "memory_used",
        lua.create_function(|lua, ()| Ok(lua.used_memory()))?,
    )?;
    module.set(
        "time_us",
        lua.create_function(|_, ()| -> mlua::Result<u64> { Ok(execution::time_us()) })?,
    )?;
    lua.globals().set("engine", module)
}

pub fn parse_address(value: &str) -> mlua::Result<super::types::Address> {
    let value = value.trim();
    let (digits, radix) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map_or((value, 10), |digits| (digits, 16));
    let address = u64::from_str_radix(digits, radix).map_err(|_| {
        mlua::Error::runtime("address must be a valid u64 decimal or 0x hexadecimal value")
    })?;
    Ok(super::types::Address::new(address))
}
