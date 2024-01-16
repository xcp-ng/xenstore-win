pub mod wmi_extra;

use std::error::Error;
use std::io::{Error as IoError};
use std::num::NonZeroU32;
use windows::core::BSTR;
use windows::Win32::System::Wmi;

pub struct XBTransaction(NonZeroU32);

#[repr(u32)]
pub enum XsOpenFlags {
    ReadOnly = 0, //xenstore_sys::XS_OPEN_READONLY,
    //SocketOnly = xenstore_sys::XS_OPEN_SOCKETONLY,
}

pub struct Xs {
    wmi_service: Wmi::IWbemServices,
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
        // py: wmi.WMI(namespace="root\\wmi")
        let wmi_service = wmi_extra::wmi_init(r#"root\wmi"#)?;

        // py: .CitrixXenStoreBase()[0]
        let enumerator = unsafe {
            wmi_service.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from("SELECT __Path, InstanceName FROM CitrixXenStoreBase"),
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
        // get the singleton
        let xs_base = objs.into_iter().next().unwrap().unwrap();
        let xs_base_class = wmi_extra::wmi_get_object(&wmi_service, "CitrixXenStoreBase")?;

        // ret ...> session id
        let ret = wmi_extra::add_session(&wmi_service, &xs_base, &xs_base_class)?;

        Ok(Xs {
            wmi_service,
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
