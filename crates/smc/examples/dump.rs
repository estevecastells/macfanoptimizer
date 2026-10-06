//! Dump every SMC key with its type and decoded value: `cargo run -p smc --example dump`
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut smc = smc::Smc::open()?;
    let keys = smc.all_keys()?;
    eprintln!("{} keys", keys.len());
    for k in keys {
        match smc.read_raw(k) {
            Ok((info, bytes)) => {
                println!("{k}  {}  {:>2}  {}", info.data_type, info.size, smc::decode(info.data_type, &bytes))
            }
            Err(e) => println!("{k}  ERR {e}"),
        }
    }
    Ok(())
}
