//! Expulsar discos en Windows: qué unidades se pueden expulsar (extraíbles, lectoras y discos
//! USB/SD/1394/imagen montada por su bus) y la expulsión con `CM_Request_Device_EjectW` sobre el
//! padre del disco del volumen. Si el sistema lo veta, el programa que lo usa sale (como mejor
//! esfuerzo) del evento 225 de Kernel-PnP del registro de Sistema.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Parent, CM_Request_Device_EjectW, CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, PNP_VETO_TYPE,
    PNP_VetoTypeUnknown, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW, OPEN_EXISTING,
};
use windows::Win32::System::EventLog::{EvtClose, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection, EvtRender, EvtRenderEventXml};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    GUID_DEVINTERFACE_CDROM, GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
};
use windows::core::{GUID, HSTRING, PCWSTR, w};
use xdir_core::shell::EjectError;

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_CDROM: u32 = 5;

/// ¿Ese bus (`STORAGE_BUS_TYPE`) es de un disco externo o una imagen montada? 1394 (4), USB (7),
/// SD (12), MMC (13) y archivo virtual (15: ISO/VHD montados).
pub fn bus_is_external(bus: u32) -> bool {
    matches!(bus, 4 | 7 | 12 | 13 | 15)
}

/// `\\.\E:` de una raíz `E:\`.
fn volume_device(root: &Path) -> Option<HSTRING> {
    let s = root.to_string_lossy();
    let letter = s.chars().next().filter(|c| c.is_ascii_alphabetic())?;
    (s.chars().nth(1) == Some(':')).then(|| HSTRING::from(format!(r"\\.\{letter}:")))
}

/// Abre un dispositivo sin pedir acceso a los datos (alcanza para las consultas).
fn open_device(path: &HSTRING) -> Option<HANDLE> {
    unsafe {
        CreateFileW(path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None).ok()
    }
}

/// Bus del disco de la unidad.
fn bus_type(root: &Path) -> Option<u32> {
    let h = open_device(&volume_device(root)?)?;
    let query = STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceProperty, QueryType: PropertyStandardQuery, ..Default::default() };
    let mut desc = STORAGE_DEVICE_DESCRIPTOR::default();
    let mut got = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const c_void),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(&mut desc as *mut _ as *mut c_void),
            size_of::<STORAGE_DEVICE_DESCRIPTOR>() as u32,
            Some(&mut got),
            None,
        )
    };
    unsafe {
        let _ = CloseHandle(h);
    }
    ok.ok().map(|()| desc.BusType.0 as u32)
}

/// De estas raíces, las que se pueden expulsar.
pub fn ejectable(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter(|r| {
            let kind = unsafe { GetDriveTypeW(&HSTRING::from(r.as_os_str())) };
            match kind {
                DRIVE_REMOVABLE | DRIVE_CDROM => true,
                DRIVE_FIXED => bus_type(r).is_some_and(bus_is_external),
                _ => false,
            }
        })
        .cloned()
        .collect()
}

/// Número de dispositivo (tipo, número) de un volumen o de una interfaz de disco.
fn device_number(path: &HSTRING) -> Option<(u32, u32)> {
    let h = open_device(path)?;
    let mut num = STORAGE_DEVICE_NUMBER::default();
    let mut got = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            None,
            0,
            Some(&mut num as *mut _ as *mut c_void),
            size_of::<STORAGE_DEVICE_NUMBER>() as u32,
            Some(&mut got),
            None,
        )
    };
    unsafe {
        let _ = CloseHandle(h);
    }
    ok.ok().map(|()| (num.DeviceType, num.DeviceNumber))
}

/// La instancia del disco (o lectora) con ese número de dispositivo.
fn disk_instance(class: &GUID, wanted: (u32, u32)) -> Option<u32> {
    unsafe {
        let set = SetupDiGetClassDevsW(Some(class), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE).ok()?;
        let mut found = None;
        let mut i = 0;
        loop {
            let mut iface = SP_DEVICE_INTERFACE_DATA { cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInterfaces(set, None, class, i, &mut iface).is_err() {
                break;
            }
            i += 1;
            // Detalle con la ruta: un búfer de u64 para que quede alineado.
            let mut buf = vec![0u64; 1024];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            let mut info = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiGetDeviceInterfaceDetailW(set, &iface, Some(detail), (buf.len() * 8) as u32, None, Some(&mut info)).is_err() {
                continue;
            }
            let path = PCWSTR((*detail).DevicePath.as_ptr());
            let path = HSTRING::from_wide(path.as_wide());
            if device_number(&path) == Some(wanted) {
                found = Some(info.DevInst);
                break;
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        found
    }
}

/// El programa del último evento 225 de Kernel-PnP («la aplicación … detuvo la expulsión»), si
/// se puede leer.
fn last_veto_process() -> Option<String> {
    unsafe {
        let query = EvtQuery(
            None,
            w!("System"),
            w!("*[System[Provider[@Name='Microsoft-Windows-Kernel-PnP'] and (EventID=225)]]"),
            EvtQueryChannelPath.0 | EvtQueryReverseDirection.0,
        )
        .ok()?;
        let mut events = [0isize; 1];
        let mut returned = 0u32;
        let next = EvtNext(query, &mut events, 2000, 0, &mut returned);
        let mut out = None;
        if next.is_ok() && returned == 1 {
            let event = windows::Win32::System::EventLog::EVT_HANDLE(events[0]);
            let mut used = 0u32;
            let mut props = 0u32;
            let _ = EvtRender(None, event, EvtRenderEventXml.0, 0, None, &mut used, &mut props);
            let mut buf = vec![0u16; used as usize / 2 + 1];
            if EvtRender(None, event, EvtRenderEventXml.0, (buf.len() * 2) as u32, Some(buf.as_mut_ptr().cast()), &mut used, &mut props).is_ok() {
                let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                out = process_from_event_xml(&String::from_utf16_lossy(&buf[..len]));
            }
            let _ = EvtClose(event);
        }
        let _ = EvtClose(query);
        out
    }
}

/// El nombre del programa (sin ruta ni «.exe») del XML de un evento 225 (`ProcessName`).
pub fn process_from_event_xml(xml: &str) -> Option<String> {
    let at = xml.find("Name='ProcessName'").or_else(|| xml.find("Name=\"ProcessName\""))?;
    let rest = &xml[at..];
    let start = rest.find('>')? + 1;
    let end = rest[start..].find('<')? + start;
    let full = rest[start..end].trim();
    let file = full.rsplit(['\\', '/']).next()?;
    let name = file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file);
    (!name.is_empty()).then(|| name.to_string())
}

/// Expulsa la unidad de `root`.
pub fn eject(root: &Path) -> Result<(), EjectError> {
    let fail = |message: &str| EjectError { busy_by: None, busy: false, message: message.to_string() };
    let device = volume_device(root).ok_or_else(|| fail("no es una unidad"))?;
    let number = device_number(&device).ok_or_else(|| fail("no se pudo abrir la unidad"))?;
    let kind = unsafe { GetDriveTypeW(&HSTRING::from(root.as_os_str())) };
    let class = if kind == DRIVE_CDROM { GUID_DEVINTERFACE_CDROM } else { GUID_DEVINTERFACE_DISK };
    let disk = disk_instance(&class, number).ok_or_else(|| fail("no se encontró el dispositivo"))?;
    let mut parent = 0u32;
    let target = if unsafe { CM_Get_Parent(&mut parent, disk, 0) } == CR_SUCCESS { parent } else { disk };
    // Se intenta un par de veces: a veces el sistema tarda en soltar el volumen.
    let mut veto = PNP_VetoTypeUnknown;
    for _ in 0..3 {
        veto = PNP_VETO_TYPE(0);
        let mut name = vec![0u16; 260];
        let r = unsafe { CM_Request_Device_EjectW(target, Some(&mut veto), Some(&mut name), 0) };
        if r == CR_SUCCESS && veto == PNP_VetoTypeUnknown {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    let busy = veto != PNP_VetoTypeUnknown;
    Err(EjectError { busy_by: busy.then(last_veto_process).flatten(), busy, message: format!("el sistema no lo permitió ({})", veto.0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_types() {
        for bus in [4, 7, 12, 13, 15] {
            assert!(bus_is_external(bus), "{bus}");
        }
        for bus in [0, 1, 3, 11, 17] {
            assert!(!bus_is_external(bus), "{bus}");
        }
    }

    #[test]
    fn event_xml_names_the_program() {
        let xml = "<Event><EventData><Data Name='ProcessId'>1234</Data><Data Name='ProcessName'>\\Device\\HarddiskVolume3\\Program Files\\Microsoft Office\\WINWORD.EXE</Data></EventData></Event>";
        assert_eq!(process_from_event_xml(xml).as_deref(), Some("WINWORD"));
        assert_eq!(process_from_event_xml("<Event/>"), None);
    }
}
