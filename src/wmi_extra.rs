use windows::core::{BSTR, w};
use windows::Win32::System::Wmi::{
    //IWbemClassObject,
    //IWbemServices,
    WBEM_FLAG_RETURN_WBEM_COMPLETE,
};
use wmi::{WMIConnection, WMIError, WMIResult};

pub fn add_session(cnx: &WMIConnection,
                   object: &crate::CitrixXenStoreBase) -> WMIResult<usize>
{
    let mut wmi_object = None;
    let object_path: &str = object.__Path.as_str();
    let object_path = BSTR::from(object_path);
    unsafe {
        cnx.svc.GetObject(&object_path,
                          WBEM_FLAG_RETURN_WBEM_COMPLETE.0 as _,
                          None,
                          Some(&mut wmi_object),
                          None)?;
    }
    let wmi_object = wmi_object.ok_or(WMIError::NullPointerResult)?;

    let mut in_params = None;
    let mut out_params = None;
    unsafe {
        wmi_object.GetMethod(w!("AddSession"), 0, &mut in_params, &mut out_params)?;
    }

    unsafe {
        cnx.svc.ExecMethod(
            &BSTR::from(&object.__Path),
            &BSTR::from("AddSession"),
            0,
            None,
            None, //Some(& in_params),
            Some(&mut out_params),
            None,
        )?;
    }

    Ok(0)
}
