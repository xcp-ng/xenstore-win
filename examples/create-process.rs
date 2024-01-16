use std::error::Error;
use windows::core::{BSTR, VARIANT, w};
use windows::Win32::System::{ Com, Rpc, Wmi };

use xenstore_win::wmi_extra::{wmi_init, wmi_get_object};

fn type_of<T>(_: &T) -> String{
    format!("{}", std::any::type_name::<T>())
}

fn main() -> Result<(), Box<dyn Error>> {
    let svc = wmi_init(r#"ROOT\CIMV2"#)?;

    // get Win32_Process object
    let mut wmi_object = wmi_get_object(&svc, "Win32_Process")?;

    // GetMethod("Create")

    // in params
    let mut in_params_class: Option<Wmi::IWbemClassObject> = None;
    unsafe { wmi_object.GetMethod(w!("Create"), 0,
                                  &mut in_params_class, std::ptr::null_mut()) }?;
    let in_params_class = in_params_class.unwrap();
    //let mut in_params: Option<Wmi::IWbemClassObject> = None;
    let ret = unsafe { in_params_class.SpawnInstance(0) };
    let in_params = ret.expect("SpawnInstance should return an instance");

    let var_command = VARIANT::from(BSTR::from("notepad.exe"));
    let ret = unsafe { in_params.Put(w!("CommandLine"), 0, &var_command, 0) };

    // ExecMethod
    let mut out_params = None;
    unsafe { svc.ExecMethod(
        &BSTR::from("Win32_Process"),
        &BSTR::from("Create"),
        Default::default(),
        None,
        &in_params,
        Some(&mut out_params),
        None,
    ) }?;
    let out_params = out_params.unwrap();
    eprintln!("out_params = {out_params:?}");

    // out params
    let mut value = VARIANT::default();
    unsafe { out_params.Get(w!("ReturnValue"), 0, &mut value, None, None) }?;
    println!("`Create` method return value: {value:?}");

    Ok(())
}
