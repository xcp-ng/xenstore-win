pub mod wmi_extra;

use std::error::Error;
use std::io::{Error as IoError};
use std::num::NonZeroU32;
use windows::core::{BSTR, VARIANT};
use windows::Win32::System::Wmi;

pub struct XBTransaction(NonZeroU32);

#[repr(u32)]
pub enum XsOpenFlags {
    ReadOnly = 0, //xenstore_sys::XS_OPEN_READONLY,
    //SocketOnly = xenstore_sys::XS_OPEN_SOCKETONLY,
}

pub struct Xs {
    wmi_service: Wmi::IWbemServices,
    xenstore_base_class: Wmi::IWbemClassObject,
    //xenstore_base_singleton: Wmi::IWbemClassObject,
    xenstore_base_path: BSTR,
}

//#[derive(Deserialize, Debug)]
//#[serde(rename_all = "PascalCase")]
//struct CitrixXenStoreBase {
//    #[serde(rename = "__Path")]
//    __path: String,
//    instance_name: String,
//}

//#[derive(Deserialize, Debug)]
//#[serde(rename_all = "PascalCase")]
//struct CitrixXenStoreSession {
//    id: String,
//    instance_name: String,
//    session_id: usize,
//}

// python:
// wmi.WMI(namespace="root\\wmi").CitrixXenStoreBase()[0].AddSession("MyNewSession")

impl Xs {
    pub fn new(_open_type: XsOpenFlags) -> Result<Self, Box<dyn Error>> {
        let wmi_service = wmi_extra::wmi_init(r#"root\wmi"#)?;

        // get all instances of .CitrixXenStoreBase
        let enumerator = unsafe {
            wmi_service.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from("SELECT __Path FROM CitrixXenStoreBase"),
                Wmi::WBEM_FLAG_FORWARD_ONLY | Wmi::WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
        }?;
        let mut objs = [None; 1];
        let res = {
            let mut return_value = 0;
            unsafe { enumerator.Next(Wmi::WBEM_INFINITE, &mut objs, &mut return_value) }
        };
        if let Err(e) = res.ok() {
            return Err(e.into());
        }
        // get the singleton instance
        let xenstore_base_singleton = objs.into_iter().next().unwrap().unwrap();

        let mut xenstore_base_path = VARIANT::default();
        unsafe { xenstore_base_singleton
                 .Get(&BSTR::from("__Path"), 0, &mut xenstore_base_path, None, None) }?;
        let xenstore_base_path = BSTR::try_from(&xenstore_base_path)?;

        let xenstore_base_class = wmi_extra::wmi_get_object(&wmi_service, "CitrixXenStoreBase")?;
        // ret ...> session id
        let ret = wmi_extra::add_session(&wmi_service, &xenstore_base_class, &xenstore_base_path)?;

        Ok(Xs {
            wmi_service,
            xenstore_base_class,
            xenstore_base_path,
        })
    }
//    pub fn read(&self, transaction: Option<XBTransaction>, path: &str) -> Result<String, IoError> {
//        Err(IoError::last_os_error())
//    }
}

impl Drop for Xs {
    fn drop(&mut self) {
        //self.close();
    }
}
