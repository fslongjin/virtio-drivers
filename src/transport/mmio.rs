use super::{DeviceStatus, DeviceType, Transport};
use crate::{align_up, queue::Descriptor, PhysAddr, PAGE_SIZE};
use core::{
    convert::{TryFrom, TryInto},
    fmt::{self, Display, Formatter},
    mem::size_of,
    ptr::NonNull,
};
use volatile::{ReadOnly, Volatile, WriteOnly};

const MAGIC_VALUE: u32 = 0x7472_6976;
pub(crate) const LEGACY_VERSION: u32 = 1;
pub(crate) const MODERN_VERSION: u32 = 2;
const CONFIG_SPACE_OFFSET: usize = 0x100;

/// The version of the VirtIO MMIO transport supported by a device.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MmioVersion {
    /// Legacy MMIO transport with page-based addressing.
    Legacy = LEGACY_VERSION,
    /// Modern MMIO transport.
    Modern = MODERN_VERSION,
}

impl TryFrom<u32> for MmioVersion {
    type Error = MmioError;

    fn try_from(version: u32) -> Result<Self, Self::Error> {
        match version {
            LEGACY_VERSION => Ok(Self::Legacy),
            MODERN_VERSION => Ok(Self::Modern),
            _ => Err(MmioError::UnsupportedVersion(version)),
        }
    }
}

impl From<MmioVersion> for u32 {
    fn from(version: MmioVersion) -> Self {
        match version {
            MmioVersion::Legacy => LEGACY_VERSION,
            MmioVersion::Modern => MODERN_VERSION,
        }
    }
}

/// An error encountered initialising a VirtIO MMIO transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MmioError {
    /// The header doesn't start with the expected magic value 0x74726976.
    BadMagic(u32),
    /// The header reports a version number that is neither 1 (legacy) nor 2 (modern).
    UnsupportedVersion(u32),
    /// The header reports a device ID of 0.
    ZeroDeviceId,
}

impl Display for MmioError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::BadMagic(magic) => write!(
                f,
                "Invalid magic value {:#010x} (expected 0x74726976).",
                magic
            ),
            Self::UnsupportedVersion(version) => {
                write!(f, "Unsupported Virtio MMIO version {}.", version)
            }
            Self::ZeroDeviceId => write!(f, "Device ID was zero."),
        }
    }
}

/// MMIO Device Register Interface, both legacy and modern.
///
/// Ref: 4.2.2 MMIO Device Register Layout and 4.2.4 Legacy interface
#[repr(C)]
pub struct VirtIOHeader {
    /// Magic value
    magic: ReadOnly<u32>,

    /// Device version number
    ///
    /// Legacy device returns value 0x1.
    version: ReadOnly<u32>,

    /// Virtio Subsystem Device ID
    device_id: ReadOnly<u32>,

    /// Virtio Subsystem Vendor ID
    vendor_id: ReadOnly<u32>,

    /// Flags representing features the device supports
    device_features: ReadOnly<u32>,

    /// Device (host) features word selection
    device_features_sel: WriteOnly<u32>,

    /// Reserved
    __r1: [ReadOnly<u32>; 2],

    /// Flags representing device features understood and activated by the driver
    driver_features: WriteOnly<u32>,

    /// Activated (guest) features word selection
    driver_features_sel: WriteOnly<u32>,

    /// Guest page size
    ///
    /// The driver writes the guest page size in bytes to the register during
    /// initialization, before any queues are used. This value should be a
    /// power of 2 and is used by the device to calculate the Guest address
    /// of the first queue page (see QueuePFN).
    legacy_guest_page_size: WriteOnly<u32>,

    /// Reserved
    __r2: ReadOnly<u32>,

    /// Virtual queue index
    ///
    /// Writing to this register selects the virtual queue that the following
    /// operations on the QueueNumMax, QueueNum, QueueAlign and QueuePFN
    /// registers apply to. The index number of the first queue is zero (0x0).
    queue_sel: WriteOnly<u32>,

    /// Maximum virtual queue size
    ///
    /// Reading from the register returns the maximum size of the queue the
    /// device is ready to process or zero (0x0) if the queue is not available.
    /// This applies to the queue selected by writing to QueueSel and is
    /// allowed only when QueuePFN is set to zero (0x0), so when the queue is
    /// not actively used.
    queue_num_max: ReadOnly<u32>,

    /// Virtual queue size
    ///
    /// Queue size is the number of elements in the queue. Writing to this
    /// register notifies the device what size of the queue the driver will use.
    /// This applies to the queue selected by writing to QueueSel.
    queue_num: WriteOnly<u32>,

    /// Used Ring alignment in the virtual queue
    ///
    /// Writing to this register notifies the device about alignment boundary
    /// of the Used Ring in bytes. This value should be a power of 2 and
    /// applies to the queue selected by writing to QueueSel.
    legacy_queue_align: WriteOnly<u32>,

    /// Guest physical page number of the virtual queue
    ///
    /// Writing to this register notifies the device about location of the
    /// virtual queue in the Guest’s physical address space. This value is
    /// the index number of a page starting with the queue Descriptor Table.
    /// Value zero (0x0) means physical address zero (0x00000000) and is illegal.
    /// When the driver stops using the queue it writes zero (0x0) to this
    /// register. Reading from this register returns the currently used page
    /// number of the queue, therefore a value other than zero (0x0) means that
    /// the queue is in use. Both read and write accesses apply to the queue
    /// selected by writing to QueueSel.
    legacy_queue_pfn: Volatile<u32>,

    /// new interface only
    queue_ready: Volatile<u32>,

    /// Reserved
    __r3: [ReadOnly<u32>; 2],

    /// Queue notifier
    queue_notify: WriteOnly<u32>,

    /// Reserved
    __r4: [ReadOnly<u32>; 3],

    /// Interrupt status
    interrupt_status: ReadOnly<u32>,

    /// Interrupt acknowledge
    interrupt_ack: WriteOnly<u32>,

    /// Reserved
    __r5: [ReadOnly<u32>; 2],

    /// Device status
    ///
    /// Reading from this register returns the current device status flags.
    /// Writing non-zero values to this register sets the status flags,
    /// indicating the OS/driver progress. Writing zero (0x0) to this register
    /// triggers a device reset. The device sets QueuePFN to zero (0x0) for
    /// all queues in the device. Also see 3.1 Device Initialization.
    status: Volatile<DeviceStatus>,

    /// Reserved
    __r6: [ReadOnly<u32>; 3],

    // new interface only since here
    queue_desc_low: WriteOnly<u32>,
    queue_desc_high: WriteOnly<u32>,

    /// Reserved
    __r7: [ReadOnly<u32>; 2],

    queue_driver_low: WriteOnly<u32>,
    queue_driver_high: WriteOnly<u32>,

    /// Reserved
    __r8: [ReadOnly<u32>; 2],

    queue_device_low: WriteOnly<u32>,
    queue_device_high: WriteOnly<u32>,

    /// Reserved
    __r9: [ReadOnly<u32>; 21],

    config_generation: ReadOnly<u32>,
}

impl VirtIOHeader {
    /// Constructs a fake VirtIO header for use in unit tests.
    #[cfg(test)]
    pub fn make_fake_header(
        version: u32,
        device_id: u32,
        vendor_id: u32,
        device_features: u32,
        queue_num_max: u32,
    ) -> Self {
        Self {
            magic: ReadOnly::new(MAGIC_VALUE),
            version: ReadOnly::new(version),
            device_id: ReadOnly::new(device_id),
            vendor_id: ReadOnly::new(vendor_id),
            device_features: ReadOnly::new(device_features),
            device_features_sel: WriteOnly::default(),
            __r1: Default::default(),
            driver_features: Default::default(),
            driver_features_sel: Default::default(),
            legacy_guest_page_size: Default::default(),
            __r2: Default::default(),
            queue_sel: Default::default(),
            queue_num_max: ReadOnly::new(queue_num_max),
            queue_num: Default::default(),
            legacy_queue_align: Default::default(),
            legacy_queue_pfn: Default::default(),
            queue_ready: Default::default(),
            __r3: Default::default(),
            queue_notify: Default::default(),
            __r4: Default::default(),
            interrupt_status: Default::default(),
            interrupt_ack: Default::default(),
            __r5: Default::default(),
            status: Volatile::new(DeviceStatus::empty()),
            __r6: Default::default(),
            queue_desc_low: Default::default(),
            queue_desc_high: Default::default(),
            __r7: Default::default(),
            queue_driver_low: Default::default(),
            queue_driver_high: Default::default(),
            __r8: Default::default(),
            queue_device_low: Default::default(),
            queue_device_high: Default::default(),
            __r9: Default::default(),
            config_generation: Default::default(),
        }
    }
}

/// MMIO Device Register Interface.
///
/// Ref: 4.2.2 MMIO Device Register Layout and 4.2.4 Legacy interface
#[derive(Debug)]
pub struct MmioTransport {
    header: NonNull<VirtIOHeader>,
    version: MmioVersion,
}

impl MmioTransport {
    /// Constructs a new VirtIO MMIO transport, or returns an error if the header reports an
    /// unsupported version.
    ///
    /// # Safety
    /// `header` must point to a properly aligned valid VirtIO MMIO region, which must remain valid
    /// for the lifetime of the transport that is returned.
    pub unsafe fn new(header: NonNull<VirtIOHeader>) -> Result<Self, MmioError> {
        let magic = header.as_ref().magic.read();
        if magic != MAGIC_VALUE {
            return Err(MmioError::BadMagic(magic));
        }
        if header.as_ref().device_id.read() == 0 {
            return Err(MmioError::ZeroDeviceId);
        }
        let version = header.as_ref().version.read().try_into()?;
        Ok(Self { header, version })
    }

    /// Gets the version of the VirtIO MMIO transport.
    pub fn version(&self) -> MmioVersion {
        self.version
    }

    /// Gets the vendor ID.
    pub fn vendor_id(&self) -> u32 {
        self.header().vendor_id.read()
    }

    fn header(&self) -> &VirtIOHeader {
        unsafe { self.header.as_ref() }
    }

    fn header_mut(&mut self) -> &mut VirtIOHeader {
        unsafe { self.header.as_mut() }
    }
}

impl Transport for MmioTransport {
    fn device_type(&self) -> DeviceType {
        match self.header().device_id.read() {
            x @ 1..=13 | x @ 16..=24 => unsafe { core::mem::transmute(x as u8) },
            _ => DeviceType::Invalid,
        }
    }

    fn read_device_features(&mut self) -> u64 {
        let header = self.header_mut();
        header.device_features_sel.write(0); // device features [0, 32)
        let mut device_features_bits = header.device_features.read().into();
        header.device_features_sel.write(1); // device features [32, 64)
        device_features_bits += (header.device_features.read() as u64) << 32;
        device_features_bits
    }

    fn write_driver_features(&mut self, driver_features: u64) {
        let header = self.header_mut();
        header.driver_features_sel.write(0); // driver features [0, 32)
        header.driver_features.write(driver_features as u32);
        header.driver_features_sel.write(1); // driver features [32, 64)
        header.driver_features.write((driver_features >> 32) as u32);
    }

    fn max_queue_size(&self) -> u32 {
        self.header().queue_num_max.read()
    }

    fn notify(&mut self, queue: u32) {
        self.header_mut().queue_notify.write(queue);
    }

    fn set_status(&mut self, status: DeviceStatus) {
        self.header_mut().status.write(status);
    }

    fn set_guest_page_size(&mut self, guest_page_size: u32) {
        match self.version {
            MmioVersion::Legacy => {
                self.header_mut()
                    .legacy_guest_page_size
                    .write(guest_page_size);
            }
            MmioVersion::Modern => {
                // No-op, modern devices don't care.
            }
        }
    }

    fn queue_set(
        &mut self,
        queue: u32,
        size: u32,
        descriptors: PhysAddr,
        driver_area: PhysAddr,
        device_area: PhysAddr,
    ) {
        match self.version {
            MmioVersion::Legacy => {
                assert_eq!(
                    driver_area - descriptors,
                    size_of::<Descriptor>() * size as usize
                );
                assert_eq!(
                    device_area - descriptors,
                    align_up(
                        size_of::<Descriptor>() * size as usize
                            + size_of::<u16>() * (size as usize + 3)
                    )
                );
                let align = PAGE_SIZE as u32;
                let pfn = (descriptors / PAGE_SIZE) as u32;
                assert_eq!(pfn as usize * PAGE_SIZE, descriptors);
                self.header_mut().queue_sel.write(queue);
                self.header_mut().queue_num.write(size);
                self.header_mut().legacy_queue_align.write(align);
                self.header_mut().legacy_queue_pfn.write(pfn);
            }
            MmioVersion::Modern => {
                self.header_mut().queue_sel.write(queue);
                self.header_mut().queue_num.write(size);
                self.header_mut().queue_desc_low.write(descriptors as u32);
                self.header_mut()
                    .queue_desc_high
                    .write((descriptors >> 32) as u32);
                self.header_mut().queue_driver_low.write(driver_area as u32);
                self.header_mut()
                    .queue_driver_high
                    .write((driver_area >> 32) as u32);
                self.header_mut().queue_device_low.write(device_area as u32);
                self.header_mut()
                    .queue_device_high
                    .write((device_area >> 32) as u32);
                self.header_mut().queue_ready.write(1);
            }
        }
    }

    fn queue_used(&mut self, queue: u32) -> bool {
        self.header_mut().queue_sel.write(queue);
        match self.version {
            MmioVersion::Legacy => self.header().legacy_queue_pfn.read() != 0,
            MmioVersion::Modern => self.header().queue_ready.read() != 0,
        }
    }

    fn ack_interrupt(&mut self) -> bool {
        let header = self.header_mut();
        let interrupt = header.interrupt_status.read();
        if interrupt != 0 {
            header.interrupt_ack.write(interrupt);
            true
        } else {
            false
        }
    }

    fn config_space(&self) -> NonNull<u64> {
        NonNull::new((self.header.as_ptr() as usize + CONFIG_SPACE_OFFSET) as _).unwrap()
    }
}
