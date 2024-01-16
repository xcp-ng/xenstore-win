use windows::core::{BSTR, Error as WinError, VARIANT, w};
use windows::Win32::System::{Com, Rpc, Wmi};

pub fn wmi_init(path: &str) -> Result<Wmi::IWbemServices, WinError> {
    unsafe { Com::CoInitializeEx(None, Com::COINIT_MULTITHREADED) }?;
    unsafe { Com::CoInitializeSecurity(None,
                                       -1, // let COM choose.
                                       None,
                                       None,
                                       Com::RPC_C_AUTHN_LEVEL_DEFAULT,
                                       Com::RPC_C_IMP_LEVEL_IMPERSONATE,
                                       None,
                                       Com::EOAC_NONE,
                                       None,
    ) }?;

    // COM locator
    let loc: Wmi::IWbemLocator = unsafe {
        Com::CoCreateInstance(&Wmi::WbemLocator, None, Com::CLSCTX_INPROC_SERVER)
    }?;
    // connection
    let svc = unsafe {
        loc.ConnectServer(
            &BSTR::from(path),
            &BSTR::new(),
            &BSTR::new(),
            &BSTR::new(),
            Wmi::WBEM_FLAG_CONNECT_USE_MAX_WAIT.0,
            &BSTR::new(),
            None,
        )
    }?;

    // "set proxy"
    unsafe {
        Com::CoSetProxyBlanket(
            &svc,
            Rpc::RPC_C_AUTHN_WINNT, // RPC_C_AUTHN_xxx
            Rpc::RPC_C_AUTHZ_NONE,  // RPC_C_AUTHZ_xxx
            None,
            Com::RPC_C_AUTHN_LEVEL_CALL,      // RPC_C_AUTHN_LEVEL_xxx
            Com::RPC_C_IMP_LEVEL_IMPERSONATE, // RPC_C_IMP_LEVEL_xxx
            None,                        // client identity
            Com::EOAC_NONE,              // proxy capabilities
        )
    }?;

    Ok(svc)
}

pub fn wmi_get_object(svc: &Wmi::IWbemServices, name: &str)
                      -> Result<Wmi::IWbemClassObject, WinError> {
    let mut wmi_object = None;
    unsafe { svc.GetObject(&BSTR::from(name),
                           Wmi::WBEM_FLAG_RETURN_WBEM_COMPLETE,
                           None,
                           Some(&mut wmi_object),
                           None) }?;
    // FIXME can this unwrap fail?  if missing driver?
    Ok(wmi_object.unwrap())
}

fn type_of<T>(_: &T) -> String{
    format!("{}", std::any::type_name::<T>())
}

pub fn add_session(svc: &Wmi::IWbemServices,
                   xs_base_class: &Wmi::IWbemClassObject,
                   xs_base_path: &BSTR) -> Result<u32, WinError>
{
    // get input params def
    let mut in_params_class: Option<Wmi::IWbemClassObject> = None;
    unsafe { xs_base_class.GetMethod(w!("AddSession"), 0,
                                     &mut in_params_class, std::ptr::null_mut()) }?;
    let in_params_class = in_params_class.unwrap();
    // fill input params
    let in_params = unsafe { in_params_class.SpawnInstance(0) }?;
    let var_session_name = VARIANT::from("MySession");
    unsafe { in_params.Put(w!("Id"), 0, &var_session_name, 0) }?;

    // method call
    let mut out_params = None;
    unsafe { svc.ExecMethod(
        xs_base_path,
        &BSTR::from("AddSession"),
        Default::default(),
        None,
        &in_params,
        Some(&mut out_params),
        None,
    ) }?;
    let out_params = out_params.unwrap();

    // output params
    let mut sid = VARIANT::default();
    unsafe { out_params.Get(w!("SessionId"), 0, &mut sid, None, None) }?;
    let sid = u32::try_from(&sid)?;
    eprintln!("sid: {:#?}", sid);

    Ok(sid)
}
