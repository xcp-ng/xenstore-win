use serde::Deserialize;
use std::error::Error;
use std::io::{Error as IoError};
use std::num::NonZeroU32;
use wmi::{COMLibrary, WMIConnection};

pub struct XBTransaction(NonZeroU32);

#[repr(u32)]
pub enum XsOpenFlags {
    ReadOnly = 0, //xenstore_sys::XS_OPEN_READONLY,
    //SocketOnly = xenstore_sys::XS_OPEN_SOCKETONLY,
}

pub struct Xs {
    wmi_connection: WMIConnection,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct CitrixXenStoreBase {
    #[serde(rename = "__Path")]
    __Path: String,
    instance_name: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct CitrixXenStoreSession {
    id: String,
    instance_name: String,
    session_id: usize,
}

// python:
// wmi.WMI(namespace="root\\wmi").CitrixXenStoreBase()[0].AddSession("MyNewSession")

impl Xs {
    pub fn new(open_type: XsOpenFlags) -> Result<Self, Box<dyn Error>> {
        // py: wmi.WMI(namespace="root\\wmi")
        let wmi_connection = WMIConnection::with_namespace_path(r#"root\wmi"#,
                                                                COMLibrary::new()?)?;
        eprintln!("WMI opened: {:p}", &wmi_connection);
        // py: .CitrixXenStoreBase()[0]
        let ret: Vec<CitrixXenStoreBase> = wmi_connection.query()?;
        let xs_base = &ret[0];

        eprintln!("Found xenstore: {} {}", xs_base.instance_name, xs_base.__Path);

        Ok(Xs {
            wmi_connection,
        })
    }
    pub fn read(&self, transaction: Option<XBTransaction>, path: &str) -> Result<String, IoError> {
        Err(IoError::last_os_error())
    }
}

impl Drop for Xs {
    fn drop(&mut self) {
        //self.close();
    }
}
