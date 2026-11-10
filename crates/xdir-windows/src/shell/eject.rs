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
    GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
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
            // Solo uno reciente (de este intento), no el de otra expulsión de hace días.
            w!("*[System[Provider[@Name='Microsoft-Windows-Kernel-PnP'] and (EventID=225) and TimeCreated[timediff(@SystemTime) <= 15000]]]"),
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

/// Cómo se expulsa una unidad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EjectPlan {
    /// Lectora de CD/DVD: se traba y desmonta el volumen y se saca el disco
    /// (`IOCTL_STORAGE_EJECT_MEDIA`); el dispositivo queda.
    Media,
    /// Imagen ISO/VHD montada (bus de archivo virtual): «Expulsar» del Shell, que la desmonta.
    ShellVerb,
    /// Disco extraíble: `CM_Request_Device_EjectW` sobre el primer nodo extraíble, subiendo desde
    /// el disco (nunca más arriba).
    Device,
}

/// Qué hacer según el tipo de unidad (`GetDriveTypeW`) y el bus del disco.
pub fn eject_plan(drive_type: u32, bus: Option<u32>) -> EjectPlan {
    match (drive_type, bus) {
        (_, Some(15)) => EjectPlan::ShellVerb,
        (DRIVE_CDROM, _) => EjectPlan::Media,
        _ => EjectPlan::Device,
    }
}

/// El primer nodo extraíble desde `start` (incluido) subiendo por los padres, hasta `max` niveles.
/// Sin ninguno extraíble, `None`: no se expulsa un controlador o concentrador de más arriba.
pub fn first_removable(start: u32, removable: &dyn Fn(u32) -> bool, parent: &dyn Fn(u32) -> Option<u32>, max: usize) -> Option<u32> {
    let mut node = start;
    for _ in 0..max {
        if removable(node) {
            return Some(node);
        }
        node = parent(node)?;
    }
    None
}

/// ¿El nodo se puede quitar? (`DN_REMOVABLE` en su estado).
fn devnode_removable(node: u32) -> bool {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{CM_DEVNODE_STATUS_FLAGS, CM_Get_DevNode_Status, CM_PROB, DN_REMOVABLE};
    let (mut status, mut problem) = (CM_DEVNODE_STATUS_FLAGS(0), CM_PROB(0));
    unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, node, 0) == CR_SUCCESS && status.0 & DN_REMOVABLE.0 != 0 }
}

fn devnode_parent(node: u32) -> Option<u32> {
    let mut parent = 0u32;
    (unsafe { CM_Get_Parent(&mut parent, node, 0) } == CR_SUCCESS).then_some(parent)
}

/// Saca el disco de una lectora: traba el volumen (si alguien lo usa, está ocupado), lo desmonta
/// y lo expulsa.
fn eject_media(root: &Path) -> Result<(), EjectError> {
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::System::Ioctl::{FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_STORAGE_EJECT_MEDIA, IOCTL_STORAGE_MEDIA_REMOVAL, PREVENT_MEDIA_REMOVAL};
    let device = volume_device(root).ok_or_else(|| fail("no es una unidad"))?;
    // Una lectora se abre solo para leer (pedir escritura falla en un CD).
    let h = unsafe { CreateFileW(&device, GENERIC_READ.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }
        .map_err(|e| fail(&e.message()))?;
    let ioctl = |code: u32| unsafe { DeviceIoControl(h, code, None, 0, None, 0, None, None) };
    // Permitir sacar el disco (un programa pudo haberlo trabado).
    let allow = |h: HANDLE| {
        let prevent = PREVENT_MEDIA_REMOVAL { PreventMediaRemoval: false };
        unsafe {
            DeviceIoControl(
                h,
                IOCTL_STORAGE_MEDIA_REMOVAL,
                Some(&prevent as *const _ as *const c_void),
                size_of::<PREVENT_MEDIA_REMOVAL>() as u32,
                None,
                0,
                None,
                None,
            )
        }
    };
    let mut locked = false;
    for _ in 0..3 {
        if ioctl(FSCTL_LOCK_VOLUME).is_ok() {
            locked = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    let result = if !locked {
        Err(EjectError { busy_by: None, busy: true, message: "la unidad está en uso".into() })
    } else {
        let _ = ioctl(FSCTL_DISMOUNT_VOLUME);
        let _ = allow(h);
        ioctl(IOCTL_STORAGE_EJECT_MEDIA).map_err(|e| fail(&e.message()))
    };
    unsafe {
        let _ = CloseHandle(h);
    }
    result
}

/// «Expulsar» del Shell sobre la unidad (imágenes ISO/VHD montadas). Corre en un hilo de trabajo:
/// inicializa COM.
fn eject_with_shell(root: &Path) -> Result<(), EjectError> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
    let item = xdir_core::shell::Item::from(root.to_path_buf());
    let result = crate::shell::win32::invoke_verb(xdir_core::sys::Handle(0), &xdir_core::fs::Location::Computer, &[item], "eject")
        .map_err(|e| fail(&e.message()));
    // Cada CoInitializeEx que anduvo lleva su CoUninitialize.
    if com {
        unsafe { CoUninitialize() };
    }
    result
}

/// ¿El veto es porque alguien lo usa? Archivos abiertos (`OutstandingOpen`), el dispositivo o
/// su controlador, una aplicación o un servicio de Windows. Los demás (no se puede quitar, falta
/// de permisos…) no se arreglan reintentando.
pub fn veto_is_busy(veto: i32) -> bool {
    matches!(veto, 3..=7)
}

fn fail(message: &str) -> EjectError {
    EjectError { busy_by: None, busy: false, message: message.to_string() }
}

/// Expulsa la unidad de `root`.
pub fn eject(root: &Path) -> Result<(), EjectError> {
    let kind = unsafe { GetDriveTypeW(&HSTRING::from(root.as_os_str())) };
    match eject_plan(kind, bus_type(root)) {
        EjectPlan::Media => return eject_media(root),
        EjectPlan::ShellVerb => return eject_with_shell(root),
        EjectPlan::Device => {}
    }
    let device = volume_device(root).ok_or_else(|| fail("no es una unidad"))?;
    let number = device_number(&device).ok_or_else(|| fail("no se pudo abrir la unidad"))?;
    let disk = disk_instance(&GUID_DEVINTERFACE_DISK, number).ok_or_else(|| fail("no se encontró el dispositivo"))?;
    let target = first_removable(disk, &devnode_removable, &devnode_parent, 4).ok_or_else(|| fail("el disco no se puede quitar"))?;
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
    let busy = veto_is_busy(veto.0);
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

    /// Lectoras: sacar el disco; imágenes montadas: el Shell; el resto, el dispositivo.
    #[test]
    fn eject_plans() {
        assert_eq!(eject_plan(DRIVE_CDROM, Some(1)), EjectPlan::Media);
        assert_eq!(eject_plan(DRIVE_CDROM, None), EjectPlan::Media);
        assert_eq!(eject_plan(DRIVE_CDROM, Some(15)), EjectPlan::ShellVerb);
        assert_eq!(eject_plan(DRIVE_FIXED, Some(15)), EjectPlan::ShellVerb);
        assert_eq!(eject_plan(DRIVE_REMOVABLE, Some(7)), EjectPlan::Device);
        assert_eq!(eject_plan(DRIVE_FIXED, Some(7)), EjectPlan::Device);
    }

    /// Se sube solo hasta el primer nodo extraíble; si no hay ninguno cerca, nada.
    #[test]
    fn walks_up_to_first_removable() {
        // 10 (disco) → 20 (USB, extraíble) → 30 (concentrador) → 40 (controlador).
        let parent = |n: u32| (n < 40).then_some(n + 10);
        assert_eq!(first_removable(10, &|n| n == 20, &parent, 4), Some(20));
        assert_eq!(first_removable(10, &|n| n == 10 || n == 20, &parent, 4), Some(10));
        assert_eq!(first_removable(10, &|_| false, &parent, 4), None);
        assert_eq!(first_removable(10, &|n| n == 40, &parent, 2), None, "no sube más de lo permitido");
    }

    #[test]
    fn busy_vetoes() {
        use windows::Win32::Devices::DeviceAndDriverInstallation::{
            PNP_VetoDevice, PNP_VetoDriver, PNP_VetoOutstandingOpen, PNP_VetoWindowsApp, PNP_VetoWindowsService,
        };
        for v in [PNP_VetoOutstandingOpen, PNP_VetoDevice, PNP_VetoDriver, PNP_VetoWindowsApp, PNP_VetoWindowsService] {
            assert!(veto_is_busy(v.0), "{v:?}");
        }
        for v in [0, 1, 2, 8, 9, 10, 11, 12] {
            assert!(!veto_is_busy(v), "{v}");
        }
    }

    #[test]
    fn event_xml_names_the_program() {
        let xml = "<Event><EventData><Data Name='ProcessId'>1234</Data><Data Name='ProcessName'>\\Device\\HarddiskVolume3\\Program Files\\Microsoft Office\\WINWORD.EXE</Data></EventData></Event>";
        assert_eq!(process_from_event_xml(xml).as_deref(), Some("WINWORD"));
        assert_eq!(process_from_event_xml("<Event/>"), None);
    }
}
