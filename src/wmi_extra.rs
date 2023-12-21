use windows::core::{BSTR, w};
use windows::Win32::System::Wmi::{
    //IWbemClassObject,
    //IWbemServices,
    WBEM_FLAG_RETURN_WBEM_COMPLETE,
};
use wmi::{WMIConnection, WMIError, WMIResult};
fn type_of<T>(_: &T) -> String{
    format!("{}", std::any::type_name::<T>())
}

pub fn add_session(cnx: &WMIConnection,
                   object: &crate::CitrixXenStoreBase) -> WMIResult<usize>
{
    let mut wmi_object = None;
    let object_path: &str = object.__path.as_str();
    let object_path = BSTR::from(object_path);
    unsafe {
        let ret = cnx.svc.GetObject(&object_path,
                                    WBEM_FLAG_RETURN_WBEM_COMPLETE.0 as _,
                                    None,
                                    Some(&mut wmi_object),
                                    None);
        eprintln!("GetObject -> {} = {:?}", type_of(&ret), ret);
        ret.expect("GetObject failure");
    }
    let wmi_object = wmi_object.ok_or(WMIError::NullPointerResult)?;
    eprintln!("wmi_object: {}", type_of(&wmi_object));

    let mut in_params = None;
    let mut out_params = None;
    unsafe {
        let ret = wmi_object.GetMethod(w!("AddSession"), 0, &mut in_params, &mut out_params);
        eprintln!("GetMethod -> {} = {:?}", type_of(&ret), ret);
        ret.expect("GetMethod failure");
    }

    unsafe {
        let ret = cnx.svc.ExecMethod(
            &BSTR::from(&object.__path),
            &BSTR::from("AddSession"),
            0,
            None,
            None, //in_params,
            Some(&mut out_params),
            None,
        );

        eprintln!("ExecMethod -> {} = {:?}", type_of(&ret), ret);
        ret.expect("ExecMethod failure");
    }

    Ok(0)
}
