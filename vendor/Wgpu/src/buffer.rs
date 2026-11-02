//! GPU buffers of typed elements.

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use pwrs::prelude::*;

use crate::device::{core_for, CoreRef, Device};
use crate::element::{self, ElementType, Target};
use crate::error::{device_error, invalid};
use crate::tasks::{self, Interrupt};

/// What a buffer may be used for.
#[psenum(name = "Wgpu.BufferUsage")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum BufferUsage {
    /// Mapped for reading on the CPU.
    MapRead,
    /// Mapped for writing on the CPU.
    MapWrite,
    /// The source of a copy.
    CopySrc,
    /// The destination of a copy or a write.
    CopyDst,
    /// An index buffer of a draw.
    Index,
    /// A vertex buffer of a draw.
    Vertex,
    /// A uniform buffer binding.
    Uniform,
    /// A storage buffer binding.
    #[default]
    Storage,
    /// The arguments of an indirect dispatch or draw.
    Indirect,
    /// The destination of resolved query results.
    QueryResolve,
    /// Input to a bottom-level acceleration structure build.
    BlasInput,
    /// Input to a top-level acceleration structure build.
    TlasInput,
}

const USAGES: [(BufferUsage, wgpu::BufferUsages); 12] = [
    (BufferUsage::MapRead, wgpu::BufferUsages::MAP_READ),
    (BufferUsage::MapWrite, wgpu::BufferUsages::MAP_WRITE),
    (BufferUsage::CopySrc, wgpu::BufferUsages::COPY_SRC),
    (BufferUsage::CopyDst, wgpu::BufferUsages::COPY_DST),
    (BufferUsage::Index, wgpu::BufferUsages::INDEX),
    (BufferUsage::Vertex, wgpu::BufferUsages::VERTEX),
    (BufferUsage::Uniform, wgpu::BufferUsages::UNIFORM),
    (BufferUsage::Storage, wgpu::BufferUsages::STORAGE),
    (BufferUsage::Indirect, wgpu::BufferUsages::INDIRECT),
    (BufferUsage::QueryResolve, wgpu::BufferUsages::QUERY_RESOLVE),
    (BufferUsage::BlasInput, wgpu::BufferUsages::BLAS_INPUT),
    (BufferUsage::TlasInput, wgpu::BufferUsages::TLAS_INPUT),
];

/// Every usage but mapping and acceleration-structure input: a buffer
/// that binds as storage or uniform, draws as vertices, indices or
/// indirect arguments, takes query results, and copies both ways.
pub(crate) fn general_usage() -> wgpu::BufferUsages {
    wgpu::BufferUsages::STORAGE
        | wgpu::BufferUsages::UNIFORM
        | wgpu::BufferUsages::VERTEX
        | wgpu::BufferUsages::INDEX
        | wgpu::BufferUsages::INDIRECT
        | wgpu::BufferUsages::QUERY_RESOLVE
        | wgpu::BufferUsages::COPY_SRC
        | wgpu::BufferUsages::COPY_DST
}

pub(crate) fn usage_flags(list: &[BufferUsage]) -> wgpu::BufferUsages {
    USAGES.iter().filter(|(u, _)| list.contains(u)).fold(wgpu::BufferUsages::empty(), |all, (_, f)| all | *f)
}

pub(crate) fn usage_list(flags: wgpu::BufferUsages) -> Vec<BufferUsage> {
    USAGES.iter().filter(|(_, f)| flags.contains(*f)).map(|(u, _)| *u).collect()
}

/// A GPU buffer of typed elements, held in Rust. Its data stays on the
/// GPU until something reads it.
#[psclass(name = "Wgpu.Buffer", mode = proxy, native_bytes = Buffer::native_bytes, columns = ["Label", "ElementType", "Length", "Size", "Adapter"])]
pub struct Buffer {
    /// The buffer's label.
    pub label: String,
    /// The type of its elements.
    pub element_type: ElementType,
    /// The number of elements it holds.
    pub length: u64,
    /// Its size in bytes, the elements' bytes rounded up to a multiple of 4.
    pub size: u64,
    /// What it may be used for.
    pub usage: Vec<BufferUsage>,
    /// The adapter of the device it lives on.
    pub adapter: String,
    #[psfield(skip)]
    pub(crate) buffer: wgpu::Buffer,
    #[psfield(skip)]
    pub(crate) core: CoreRef,
}

impl Buffer {
    fn native_bytes(&self) -> usize {
        self.size as usize
    }

    pub(crate) fn wrap(core: &CoreRef, buffer: wgpu::Buffer, element_type: ElementType, length: u64, label: String) -> Buffer {
        Buffer {
            label,
            element_type,
            length,
            size: buffer.size(),
            usage: usage_list(buffer.usage()),
            adapter: core.info.name.clone(),
            buffer,
            core: core.clone(),
        }
    }

    /// The element range `start..start + count` in bytes, checked against
    /// the buffer's length.
    fn byte_range(&self, start: u64, count: Option<u64>) -> PsResult<(u64, u64)> {
        if start > self.length {
            return Err(invalid("WgpuRange", format!("element {start} is past the end of a buffer of {} elements", self.length)));
        }
        let count = match count {
            Some(n) => n,
            None => self.length - start,
        };
        if start + count > self.length {
            return Err(invalid("WgpuRange", format!("elements {start} to {} run past the end of a buffer of {} elements", start + count, self.length)));
        }
        let size = self.element_type.size();
        Ok((start * size, count * size))
    }

    /// The byte offset and size of `count` source elements from element
    /// `source_offset`, and the buffer and byte offset of element
    /// `destination_offset` of `destination`, for a copy between them.
    fn copy_range(&self, destination: &PsProxy<Buffer>, source_offset: Option<u64>, destination_offset: Option<u64>, count: Option<u64>) -> PsResult<CopyRange> {
        let (from, size) = self.byte_range(source_offset.unwrap_or(0), count)?;
        destination.with(|dst| {
            if dst.core.id != self.core.id {
                return Err(invalid("WgpuOtherDevice", "a buffer copy runs between buffers of one device"));
            }
            Ok(CopyRange { from, size, target: dst.buffer.clone(), to: destination_offset.unwrap_or(0) * dst.element_type.size() })
        })?
    }

    /// Overwrites elements from element `offset` (0 when omitted) with
    /// `data`, converted to the buffer's element type.
    fn write_data(&self, data: &PsObject, offset: Option<u64>, stop: Option<&dyn Interrupt>) -> PsResult<()> {
        let start = offset.unwrap_or(0);
        element::with_host_bytes(data, Some(self.element_type), |_, bytes| {
            let count = bytes.len() as u64 / self.element_type.size();
            let (offset, _) = self.byte_range(start, Some(count))?;
            write_bytes(&self.core, &self.buffer, offset, bytes, stop)
        })
    }

    /// The bytes of every element.
    fn whole(&self) -> u64 {
        self.length * self.element_type.size()
    }

    /// Every element, as a .NET array of `element_type` or the buffer's own.
    pub(crate) fn read_all(&self, element_type: Option<ElementType>) -> PsResult<PsObject> {
        let ty = element_type.unwrap_or(self.element_type);
        read_new(&self.core, &self.buffer, 0, self.whole(), ty, false, None)
    }
}

/// Where a copy between two buffers reads and writes.
struct CopyRange {
    from: u64,
    size: u64,
    target: wgpu::Buffer,
    to: u64,
}

/// Sets every element of `buffer`, `length` elements, to the one element
/// `one` holds the bytes of; an element of zero bytes clears the buffer.
fn fill_elements(core: &CoreRef, buffer: &wgpu::Buffer, one: &[u8], length: u64, stop: Option<&dyn Interrupt>) -> PsResult<()> {
    if one.iter().all(|b| *b == 0) {
        return clear(core, buffer, stop);
    }
    write_bytes(core, buffer, 0, &one.repeat(length as usize), stop)
}

#[psmethods]
impl Buffer {
    /// Copies the buffer back and returns its elements as a .NET array of
    /// their type; `elementType` reads the same bytes as another type.
    pub fn read(&self, element_type: Option<ElementType>) -> PsResult<PsObject> {
        self.read_all(element_type)
    }

    /// Read on a thread of its own: returns at once with a task whose
    /// result is the array.
    pub fn read_async(&self, element_type: Option<ElementType>, task: PsTask<PsObject>) -> PsResult<()> {
        let ty = element_type.unwrap_or(self.element_type);
        read_new_on_thread(&self.core, &self.buffer, 0, self.whole(), ty, false, task)
    }

    /// Copies `count` elements from element `start` back, `count` running
    /// to the end when omitted.
    pub fn read_range(&self, start: u64, count: Option<u64>) -> PsResult<PsObject> {
        let (offset, size) = self.byte_range(start, count)?;
        read_new(&self.core, &self.buffer, offset, size, self.element_type, false, None)
    }

    /// ReadRange on a thread of its own: returns at once with a task whose
    /// result is the elements.
    pub fn read_range_async(&self, start: u64, count: Option<u64>, task: PsTask<PsObject>) -> PsResult<()> {
        let (offset, size) = self.byte_range(start, count)?;
        read_new_on_thread(&self.core, &self.buffer, offset, size, self.element_type, false, task)
    }

    /// Copies the buffer back into a `Memory<T>` handed to .NET without a
    /// second copy (a `T[]` on Windows PowerShell).
    pub fn read_memory(&self) -> PsResult<PsObject> {
        read_new(&self.core, &self.buffer, 0, self.whole(), self.element_type, true, None)
    }

    /// ReadMemory on a thread of its own: returns at once with a task whose
    /// result is the `Memory<T>`.
    pub fn read_memory_async(&self, task: PsTask<PsObject>) -> PsResult<()> {
        read_new_on_thread(&self.core, &self.buffer, 0, self.whole(), self.element_type, true, task)
    }

    /// Copies the buffer back into `destination` from its index 0: an
    /// array of the type Read returns, holding at least the buffer's
    /// elements. The elements past them keep their values.
    pub fn read_into(&self, destination: PsObject) -> PsResult<()> {
        let mut target = Target::given(self.element_type, self.length, destination)?;
        read_to(&self.core, &self.buffer, 0, self.whole(), &mut target, None)
    }

    /// ReadInto on a thread of its own: `destination` is checked now, and
    /// the task ends when it holds the elements.
    pub fn read_into_async(&self, destination: PsObject, task: PsTask<()>) -> PsResult<()> {
        read_to_on_thread(&self.core, &self.buffer, 0, self.whole(), Target::given(self.element_type, self.length, destination)?, task, |_| Ok(()))
    }

    /// Overwrites elements from element `offset` (0 when omitted) with
    /// `data`, converted to the buffer's element type.
    pub fn write(&self, data: PsObject, offset: Option<u64>) -> PsResult<()> {
        self.write_data(&data, offset, None)
    }

    /// Write with the writing on a thread of its own: `data` is converted
    /// now, and the task ends when it is written.
    pub fn write_async(&self, data: PsObject, offset: Option<u64>, task: PsTask<()>) -> PsResult<()> {
        let start = offset.unwrap_or(0);
        let (offset, bytes) = element::with_host_bytes(&data, Some(self.element_type), |_, bytes| {
            let count = bytes.len() as u64 / self.element_type.size();
            let (offset, _) = self.byte_range(start, Some(count))?;
            Ok((offset, bytes.to_vec()))
        })?;
        let (core, buffer) = (self.core.clone(), self.buffer.clone());
        tasks::spawn(task, move |stop| write_bytes(&core, &buffer, offset, &bytes, Some(stop)), Ok)
    }

    /// Sets every element to `value`, a number converted to the element
    /// type as each number of untyped data is.
    pub fn fill(&self, value: PsObject) -> PsResult<()> {
        let one = element::scalar_bytes(self.element_type, &value)?;
        fill_elements(&self.core, &self.buffer, &one, self.length, None)
    }

    /// Fill with the filling on a thread of its own: `value` is converted
    /// now, and the task ends when every element holds it.
    pub fn fill_async(&self, value: PsObject, task: PsTask<()>) -> PsResult<()> {
        let one = element::scalar_bytes(self.element_type, &value)?;
        let (core, buffer, length) = (self.core.clone(), self.buffer.clone(), self.length);
        tasks::spawn(task, move |stop| fill_elements(&core, &buffer, &one, length, Some(stop)), Ok)
    }

    /// Sets every byte of the buffer to zero.
    pub fn clear(&self) -> PsResult<()> {
        clear(&self.core, &self.buffer, None)
    }

    /// Clear on a thread of its own: returns at once with a task that ends
    /// when every byte is zero.
    pub fn clear_async(&self, task: PsTask<()>) -> PsResult<()> {
        let (core, buffer) = (self.core.clone(), self.buffer.clone());
        tasks::spawn(task, move |stop| clear(&core, &buffer, Some(stop)), Ok)
    }

    /// Copies `count` elements from element `sourceOffset` of this buffer
    /// to element `destinationOffset` of `destination`, as bytes.
    pub fn copy_to(&self, destination: PsProxy<Buffer>, source_offset: Option<u64>, destination_offset: Option<u64>, count: Option<u64>) -> PsResult<()> {
        let r = self.copy_range(&destination, source_offset, destination_offset, count)?;
        copy(&self.core, &self.buffer, r.from, &r.target, r.to, r.size, None)
    }

    /// CopyTo on a thread of its own: returns at once with a task that ends
    /// when the elements are copied.
    pub fn copy_to_async(&self, destination: PsProxy<Buffer>, source_offset: Option<u64>, destination_offset: Option<u64>, count: Option<u64>, task: PsTask<()>) -> PsResult<()> {
        let r = self.copy_range(&destination, source_offset, destination_offset, count)?;
        let (core, source) = (self.core.clone(), self.buffer.clone());
        tasks::spawn(task, move |stop| copy(&core, &source, r.from, &r.target, r.to, r.size, Some(stop)), Ok)
    }

    /// Elements `start` to `start + count` (to the end when `count` is
    /// omitted), bound in place of the whole buffer. A binding's offset
    /// must be a multiple of the device's MinStorageBufferOffsetAlignment,
    /// or of MinUniformBufferOffsetAlignment for a uniform.
    pub fn slice(&self, start: u64, count: Option<u64>) -> PsResult<BufferSlice> {
        let (offset, size) = self.byte_range(start, count)?;
        Ok(BufferSlice {
            label: self.label.clone(),
            start,
            length: size / self.element_type.size(),
            element_type: self.element_type,
            buffer: self.buffer.clone(),
            core: self.core.clone(),
            offset,
            size,
        })
    }

    /// The same GPU memory seen as elements of `elementType`.
    pub fn reinterpret(&self, element_type: ElementType) -> PsResult<Buffer> {
        let length = self.length * self.element_type.size() / element_type.size();
        Ok(Buffer::wrap(&self.core, self.buffer.clone(), element_type, length, self.label.clone()))
    }

    /// Frees the buffer's GPU memory now; the object stops working.
    pub fn destroy(&self) -> PsResult<()> {
        self.buffer.destroy();
        Ok(())
    }
}

/// A range of a buffer's elements, bound in place of the whole buffer.
#[psclass(name = "Wgpu.BufferSlice", mode = proxy, columns = ["Label", "Start", "Length", "ElementType"])]
pub struct BufferSlice {
    /// The buffer's label.
    pub label: String,
    /// The first element of the range.
    pub start: u64,
    /// The number of elements in the range.
    pub length: u64,
    /// The type of the elements.
    pub element_type: ElementType,
    #[psfield(skip)]
    pub(crate) buffer: wgpu::Buffer,
    #[psfield(skip)]
    pub(crate) core: CoreRef,
    #[psfield(skip)]
    pub(crate) offset: u64,
    #[psfield(skip)]
    pub(crate) size: u64,
}

#[psmethods]
impl BufferSlice {
    /// Copies the range back and returns its elements.
    pub fn read(&self) -> PsResult<PsObject> {
        read_new(&self.core, &self.buffer, self.offset, self.size, self.element_type, false, None)
    }

    /// Read on a thread of its own: returns at once with a task whose
    /// result is the elements.
    pub fn read_async(&self, task: PsTask<PsObject>) -> PsResult<()> {
        read_new_on_thread(&self.core, &self.buffer, self.offset, self.size, self.element_type, false, task)
    }

    /// Copies the range back into `destination` from its index 0: an array
    /// of the type Read returns, holding at least the range's elements. The
    /// elements past them keep their values.
    pub fn read_into(&self, destination: PsObject) -> PsResult<()> {
        let mut target = Target::given(self.element_type, self.length, destination)?;
        read_to(&self.core, &self.buffer, self.offset, self.size, &mut target, None)
    }

    /// ReadInto on a thread of its own: `destination` is checked now, and
    /// the task ends when it holds the elements.
    pub fn read_into_async(&self, destination: PsObject, task: PsTask<()>) -> PsResult<()> {
        read_to_on_thread(&self.core, &self.buffer, self.offset, self.size, Target::given(self.element_type, self.length, destination)?, task, |_| Ok(()))
    }
}

/// A new buffer of at least `size` bytes, rounded up to a multiple of 4,
/// holding `contents` at its start when given and zeros otherwise.
pub(crate) fn create_raw(core: &CoreRef, size: u64, usage: wgpu::BufferUsages, label: &str, contents: Option<&[u8]>) -> PsResult<wgpu::Buffer> {
    let padded = size.max(4).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
    if padded > core.limits.max_buffer_size {
        return Err(invalid(
            "WgpuBufferTooLarge",
            format!("{size} bytes is more than the largest buffer {} creates, {} bytes", core.info.name, core.limits.max_buffer_size),
        ));
    }
    core.scoped("WgpuBuffer", "the buffer cannot be created", || -> PsResult<wgpu::Buffer> {
        let buffer = core.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: padded, usage, mapped_at_creation: contents.is_some() });
        if let Some(bytes) = contents {
            {
                let mut view = buffer.slice(..).get_mapped_range_mut().map_err(|e| device_error("WgpuBuffer", "the new buffer cannot be written", e))?;
                view.slice(..bytes.len()).copy_from_slice(bytes);
            }
            buffer.unmap();
        }
        Ok(buffer)
    })?
}

/// A buffer holding `data` as `element_type`, or as its own type.
pub(crate) fn upload(core: &CoreRef, data: &PsObject, element_type: Option<ElementType>, usage: Option<Vec<BufferUsage>>, label: Option<String>) -> PsResult<Buffer> {
    let usage = match &usage {
        Some(list) => usage_flags(list),
        None => general_usage(),
    };
    element::with_host_bytes(data, element_type, |ty, bytes| {
        if bytes.is_empty() {
            return Err(invalid("WgpuEmpty", "a buffer needs at least one element"));
        }
        let label = label.unwrap_or_else(|| format!("{} x {:?}", bytes.len() as u64 / ty.size(), ty));
        let buffer = create_raw(core, bytes.len() as u64, usage, &label, Some(bytes))?;
        Ok(Buffer::wrap(core, buffer, ty, bytes.len() as u64 / ty.size(), label))
    })
}

/// A buffer of `length` zeroed elements, Float32 when no type is given.
pub(crate) fn zeroed(core: &CoreRef, length: u64, element_type: Option<ElementType>, usage: Option<Vec<BufferUsage>>, label: Option<String>) -> PsResult<Buffer> {
    if length == 0 {
        return Err(invalid("WgpuEmpty", "a buffer needs at least one element"));
    }
    let ty = element_type.unwrap_or(ElementType::Float32);
    let size = length
        .checked_mul(ty.size())
        .ok_or_else(|| invalid("WgpuBufferTooLarge", format!("{length} elements of {ty:?} is more bytes than a buffer can address")))?;
    let usage = match &usage {
        Some(list) => usage_flags(list),
        None => general_usage(),
    };
    let label = label.unwrap_or_else(|| format!("{length} x {ty:?}"));
    let buffer = create_raw(core, size, usage, &label, None)?;
    Ok(Buffer::wrap(core, buffer, ty, length, label))
}

fn align_down(v: u64) -> u64 {
    v & !(wgpu::COPY_BUFFER_ALIGNMENT - 1)
}

fn align_up(v: u64) -> u64 {
    v.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
}

/// The state of a staging buffer's map request, as its callback leaves it.
const MAP_PENDING: u8 = 0;
const MAP_DONE: u8 = 1;
const MAP_FAILED: u8 = 2;

/// The bytes of a page, the span within which a read's staging bytes are
/// placed to start where their destination does.
const PAGE: u64 = 4096;

/// The size from which a read's staging bytes start where their
/// destination does within a page. Placing a read costs 20 to 80 us, and a
/// copy between unequal offsets can run many times slower from 8 MiB up.
const ALIGN_FROM: u64 = 1 << 20;

/// Bytes a read copied into a staging buffer the CPU has mapped, read
/// where they are; the staging buffer is unmapped when this is dropped.
pub(crate) struct Mapped {
    /// The staging buffer; none for a read of no bytes.
    staging: Option<wgpu::Buffer>,
    /// The range of the staging buffer that is mapped.
    mapped: Range<u64>,
    /// Where the bytes start in the mapped range, and how many there are.
    from: usize,
    size: usize,
}

impl Mapped {
    /// What `f` returns for the bytes, read where the CPU mapped them.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&[u8]) -> R) -> PsResult<R> {
        let Some(staging) = &self.staging else {
            return Ok(f(&[]));
        };
        let view = staging.slice(self.mapped.clone()).get_mapped_range().map_err(|e| device_error("WgpuMap", "the mapped buffer cannot be read", e))?;
        Ok(f(&view[self.from..self.from + self.size]))
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        if let Some(staging) = &self.staging {
            staging.unmap();
        }
    }
}

/// `size` bytes from byte `offset` of `buffer`, copied as one Vec through
/// a staging buffer the CPU maps.
pub(crate) fn read_bytes(core: &CoreRef, buffer: &wgpu::Buffer, offset: u64, size: u64, stop: Option<&dyn Interrupt>) -> PsResult<Vec<u8>> {
    read_mapped(core, buffer, offset, size, stop)?.with(<[u8]>::to_vec)
}

/// `size` bytes from byte `offset` of `buffer`, copied into a staging
/// buffer the CPU maps and left there, so they are copied once more only
/// into what they become.
pub(crate) fn read_mapped(core: &CoreRef, buffer: &wgpu::Buffer, offset: u64, size: u64, stop: Option<&dyn Interrupt>) -> PsResult<Mapped> {
    read_mapped_at(core, buffer, offset, size, None, stop)
}

/// [`read_mapped`], with the bytes held from `page` bytes into a page of
/// the CPU's address space, or up to 7 bytes past, when `page` is given:
/// where the copy out of the mapping writes its first byte. Some processes
/// copy many times slower between unequal offsets within a page.
fn read_mapped_at(core: &CoreRef, buffer: &wgpu::Buffer, offset: u64, size: u64, page: Option<u64>, stop: Option<&dyn Interrupt>) -> PsResult<Mapped> {
    if size == 0 {
        return Ok(Mapped { staging: None, mapped: 0..0, from: 0, size: 0 });
    }
    let start = align_down(offset);
    let end = align_up(offset + size).min(buffer.size());
    let window = end - start;
    let from = offset - start;
    let room = if page.is_some() { PAGE } else { 0 };
    let staging = core.scoped("WgpuRead", "the buffer cannot be read back", || {
        core.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Wgpu staging"),
            size: window + room,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    })?;
    let pad = match page {
        Some(page) => {
            let base = mapped_base(core, &staging, stop)?;
            ((page + 2 * PAGE - base % PAGE - from) % PAGE).next_multiple_of(wgpu::MAP_ALIGNMENT) % PAGE
        }
        None => 0,
    };
    let copy = core.scoped("WgpuRead", "the buffer cannot be read back", || {
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu read") });
        encoder.copy_buffer_to_buffer(buffer, start, &staging, pad, window);
        encoder.finish()
    })?;
    let index = core.submit_as("WgpuRead", "the buffer cannot be read back", vec![copy], stop)?;
    let mapped = pad..pad + window;
    let state = request_map(staging.slice(mapped.clone()));
    core.wait_until(&index, stop, || state.load(Ordering::Acquire) != MAP_PENDING)?;
    if state.load(Ordering::Acquire) != MAP_DONE {
        return Err(device_error("WgpuMap", "the buffer cannot be read back", wgpu::BufferAsyncError));
    }
    Ok(Mapped { staging: Some(staging), mapped, from: from as usize, size: size as usize })
}

/// Asks wgpu to map `slice` for reading, and returns the state its
/// callback leaves, which unparks the calling thread when it is set.
fn request_map(slice: wgpu::BufferSlice<'_>) -> Arc<AtomicU8> {
    let state = Arc::new(AtomicU8::new(MAP_PENDING));
    let settled = state.clone();
    let reader = std::thread::current();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let outcome = match result {
            Ok(()) => MAP_DONE,
            Err(wgpu::BufferAsyncError) => MAP_FAILED,
        };
        settled.store(outcome, Ordering::Release);
        reader.unpark();
    });
    state
}

/// The address of byte 0 of `staging`, a staging buffer no copy has
/// written yet, in the CPU's address space, read by mapping its first 8
/// bytes and unmapping them. wgpu zeroes what it maps of a buffer nothing has
/// written, so no more of it is mapped.
fn mapped_base(core: &CoreRef, staging: &wgpu::Buffer, stop: Option<&dyn Interrupt>) -> PsResult<u64> {
    let first = staging.slice(..wgpu::MAP_ALIGNMENT);
    let state = request_map(first);
    loop {
        match state.load(Ordering::Acquire) {
            MAP_DONE => break,
            MAP_FAILED => return Err(device_error("WgpuMap", "the buffer cannot be read back", wgpu::BufferAsyncError)),
            _ => {}
        }
        if stop.is_some_and(|s| s.interrupted()) {
            return Err(tasks::stopped());
        }
        if let Err(e) = core.device.poll(wgpu::PollType::Poll) {
            return Err(device_error("WgpuPoll", "waiting for the device failed", e));
        }
        if state.load(Ordering::Acquire) == MAP_PENDING {
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }
    let base = first.get_mapped_range().map_err(|e| device_error("WgpuMap", "the mapped buffer cannot be read", e))?.as_ptr() as u64;
    staging.unmap();
    Ok(base)
}

/// Where in a page a read of `size` bytes into `target` is placed: where
/// the target's elements begin, for a read of at least [`ALIGN_FROM`].
fn placement(size: u64, target: &Target) -> PsResult<Option<u64>> {
    if size < ALIGN_FROM {
        return Ok(None);
    }
    Ok(Some(target.address()? as u64 % PAGE))
}

/// Copies `size` bytes from byte `offset` of `buffer` into `target`,
/// through a staging buffer that holds them, for a read of at least
/// [`ALIGN_FROM`], from where the target's elements begin within a page.
fn read_to(core: &CoreRef, buffer: &wgpu::Buffer, offset: u64, size: u64, target: &mut Target, stop: Option<&dyn Interrupt>) -> PsResult<()> {
    let page = placement(size, target)?;
    read_mapped_at(core, buffer, offset, size, page, stop)?.with(|bytes| target.fill(bytes))?
}

/// [`read_to`] on a thread of its own, settling `task` with what `finish`
/// returns for the filled target.
fn read_to_on_thread<T: IntoPs + 'static>(core: &CoreRef, buffer: &wgpu::Buffer, offset: u64, size: u64, mut target: Target, task: PsTask<T>, finish: impl FnOnce(Target) -> PsResult<T> + Send + 'static) -> PsResult<()> {
    let page = placement(size, &target)?;
    let (core, buffer) = (core.clone(), buffer.clone());
    tasks::spawn(task, move |stop| read_mapped_at(&core, &buffer, offset, size, page, Some(stop)), move |mapped| {
        mapped.with(|bytes| target.fill(bytes))??;
        finish(target)
    })
}

/// A new array of `ty`, or a `Memory<T>` when `memory` is set, for the
/// elements of `size` bytes.
fn new_target(ty: ElementType, size: u64, memory: bool) -> PsResult<Target> {
    if memory { Target::memory(ty, size / ty.size()) } else { Target::array(ty, size / ty.size()) }
}

/// The bytes of a read as a new array of `ty`, or a new `Memory<T>` when
/// `memory` is set, made after the read.
fn made_from(ty: ElementType, memory: bool, bytes: &[u8]) -> PsResult<PsObject> {
    if memory { element::to_clr_memory(ty, bytes) } else { element::to_clr_array(ty, bytes) }
}

/// `size` bytes from byte `offset` of `buffer` as a new array of `ty`, or
/// a new `Memory<T>` when `memory` is set (an array on .NET Framework). A
/// read of at least [`ALIGN_FROM`] makes it first and places the read where
/// its elements begin within a page; a smaller one makes it from the bytes.
pub(crate) fn read_new(core: &CoreRef, buffer: &wgpu::Buffer, offset: u64, size: u64, ty: ElementType, memory: bool, stop: Option<&dyn Interrupt>) -> PsResult<PsObject> {
    if size < ALIGN_FROM {
        return read_mapped(core, buffer, offset, size, stop)?.with(|bytes| made_from(ty, memory, bytes))?;
    }
    let mut target = new_target(ty, size, memory)?;
    read_to(core, buffer, offset, size, &mut target, stop)?;
    target.into_ps()
}

/// [`read_new`] on a thread of its own, settling `task` with the array or
/// `Memory<T>`.
fn read_new_on_thread(core: &CoreRef, buffer: &wgpu::Buffer, offset: u64, size: u64, ty: ElementType, memory: bool, task: PsTask<PsObject>) -> PsResult<()> {
    if size >= ALIGN_FROM {
        return read_to_on_thread(core, buffer, offset, size, new_target(ty, size, memory)?, task, Target::into_ps);
    }
    let (core, buffer) = (core.clone(), buffer.clone());
    tasks::spawn(task, move |stop| read_mapped(&core, &buffer, offset, size, Some(stop)), move |mapped| mapped.with(|bytes| made_from(ty, memory, bytes))?)
}

/// Writes `bytes` at byte `offset` of `buffer`. A write that starts or
/// ends inside a 4-byte word reads that word back first, so the bytes
/// around it keep their values.
pub(crate) fn write_bytes(core: &CoreRef, buffer: &wgpu::Buffer, offset: u64, bytes: &[u8], stop: Option<&dyn Interrupt>) -> PsResult<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let len = bytes.len() as u64;
    if offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT) && len.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT) {
        return core.scoped("WgpuWrite", "the buffer cannot be written", || core.queue.write_buffer(buffer, offset, bytes));
    }
    let start = align_down(offset);
    let end = align_up(offset + len);
    let mut window = read_bytes(core, buffer, start, end - start, stop)?;
    let from = (offset - start) as usize;
    window[from..from + bytes.len()].copy_from_slice(bytes);
    core.scoped("WgpuWrite", "the buffer cannot be written", || core.queue.write_buffer(buffer, start, &window))
}

/// Zeroes the whole buffer on the GPU.
pub(crate) fn clear(core: &CoreRef, buffer: &wgpu::Buffer, stop: Option<&dyn Interrupt>) -> PsResult<()> {
    let commands = core.scoped("WgpuClear", "the buffer cannot be cleared", || {
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu clear") });
        encoder.clear_buffer(buffer, 0, None);
        encoder.finish()
    })?;
    core.submit_and_wait(vec![commands], stop)
}

/// Copies `size` bytes between buffers on the GPU. Offsets and size must
/// be multiples of 4 bytes, as wgpu copies whole words.
pub(crate) fn copy(core: &CoreRef, source: &wgpu::Buffer, from: u64, destination: &wgpu::Buffer, to: u64, size: u64, stop: Option<&dyn Interrupt>) -> PsResult<()> {
    for (what, v) in [("source offset", from), ("destination offset", to), ("size", size)] {
        if v % wgpu::COPY_BUFFER_ALIGNMENT != 0 {
            return Err(invalid("WgpuCopyAlignment", format!("a buffer copy moves whole 4-byte words; the {what}, {v} bytes, is not a multiple of 4")));
        }
    }
    let commands = core.scoped("WgpuCopy", "the copy is invalid", || {
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu copy") });
        encoder.copy_buffer_to_buffer(source, from, destination, to, size);
        encoder.finish()
    })?;
    core.submit_and_wait(vec![commands], stop)
}

/// Creates a GPU buffer from an array of any numeric type, or of a length.
///
/// The buffer stays on the device until something reads it. Without
/// -Type it takes the array's own element type (Float32 for an untyped
/// array); with -Type the values are converted.
///
/// # Examples
/// $buf = New-WgpuBuffer ([float[]](1..1024))
/// $buf = New-WgpuBuffer -Length 4096 -Type UInt32
/// 1..100 | New-WgpuBuffer -Type Int32
#[cmdlet(verb = "New", noun = "WgpuBuffer", alias = ["New-GpuBuffer"], default_parameter_set = "Data", output = ["Wgpu.Buffer"])]
#[derive(Default)]
pub struct NewWgpuBuffer {
    /// The values: an array of any numeric type, a number, or numbers piped in.
    #[param(position = 0, value_from_pipeline, raw, set = "Data")]
    pub data: Option<PsObject>,
    /// The number of zeroed elements.
    #[param(mandatory, set = "Length")]
    pub length: u64,
    /// The element type; the array's own type, or Float32, when omitted.
    #[param(alias = ["Type"])]
    pub element_type: Option<ElementType>,
    /// What the buffer may be used for; every usage but mapping when omitted.
    #[param]
    pub usage: Option<Vec<BufferUsage>>,
    /// The buffer's label.
    #[param]
    pub label: Option<String>,
    /// The device to create it on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
    collected: Vec<PsObject>,
}

impl Cmdlet for NewWgpuBuffer {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        if let Some(data) = &self.data {
            self.collected.push(data.clone());
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let core = core_for(&self.device, ps)?;
        if self.collected.is_empty() {
            return ps.write(zeroed(&core, self.length, self.element_type, self.usage.clone(), self.label.clone())?);
        }
        if self.collected.len() == 1 {
            return ps.write(upload(&core, &self.collected[0], self.element_type, self.usage.clone(), self.label.clone())?);
        }
        let mut ty = self.element_type;
        let mut bytes = Vec::new();
        for item in &self.collected {
            element::with_host_bytes(item, ty, |t, b| {
                ty = Some(t);
                bytes.extend_from_slice(b);
                Ok(())
            })?;
        }
        let ty = ty.unwrap_or(ElementType::Float32);
        let usage = match &self.usage {
            Some(list) => usage_flags(list),
            None => general_usage(),
        };
        let length = bytes.len() as u64 / ty.size();
        let label = self.label.clone().unwrap_or_else(|| format!("{length} x {ty:?}"));
        let raw = create_raw(&core, bytes.len() as u64, usage, &label, Some(&bytes))?;
        ps.write(Buffer::wrap(&core, raw, ty, length, label))
    }
}

/// Copies a buffer back from the GPU and writes its elements as one array.
///
/// With -Destination it copies them into an array the caller holds
/// instead, from its index 0, and writes nothing unless -PassThru is
/// given.
///
/// # Examples
/// Read-WgpuBuffer $buf
/// $buf | Read-WgpuBuffer -Offset 10 -Count 5
/// Read-WgpuBuffer $buf -Type UInt32
/// Read-WgpuBuffer $buf -Enumerate | Measure-Object -Sum
/// Read-WgpuBuffer $buf -Destination $values
#[cmdlet(verb = "Read", noun = "WgpuBuffer", alias = ["Read-GpuBuffer"], default_parameter_set = "New", output = ["System.Array"])]
#[derive(Default)]
pub struct ReadWgpuBuffer {
    /// The buffer to read.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub buffer: Option<PsProxy<Buffer>>,
    /// The first element to read; 0 when omitted.
    #[param]
    pub offset: Option<u64>,
    /// The number of elements to read; to the end when omitted.
    #[param]
    pub count: Option<u64>,
    /// Read the bytes as this element type instead of the buffer's own.
    #[param(alias = ["Type"])]
    pub element_type: Option<ElementType>,
    /// Write a Memory<T> over the values instead of an array (an array on Windows PowerShell).
    #[param(set = "New")]
    pub as_memory: bool,
    /// Write each element to the pipeline rather than one array.
    #[param(set = "New")]
    pub enumerate: bool,
    /// The array to copy the elements into, from its index 0: of the type the read gives, float[] for Float16, holding at least the elements read. The elements past them keep their values.
    #[param(mandatory, raw, set = "Destination")]
    pub destination: Option<PsObject>,
    /// With -Destination, write the filled array as one object.
    #[param(set = "Destination")]
    pub pass_thru: bool,
}

impl Cmdlet for ReadWgpuBuffer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(proxy) = &self.buffer else {
            return Err(invalid("WgpuNoBuffer", "no buffer was given"));
        };
        let (core, raw, ty, range) = proxy.with(|b| -> PsResult<_> {
            let range = b.byte_range(self.offset.unwrap_or(0), self.count)?;
            Ok((b.core.clone(), b.buffer.clone(), b.element_type, range))
        })??;
        let ty = self.element_type.unwrap_or(ty);
        if let Some(destination) = &self.destination {
            let mut target = Target::given(ty, range.1 / ty.size(), destination.clone())?;
            read_to(&core, &raw, range.0, range.1, &mut target, Some(ps))?;
            return if self.pass_thru { ps.write_object(&target.into_ps()?) } else { Ok(()) };
        }
        let values = read_new(&core, &raw, range.0, range.1, ty, self.as_memory, Some(ps))?;
        if self.enumerate {
            ps.write_enumerated(&values)
        } else {
            ps.write_object(&values)
        }
    }
}

/// Overwrites elements of a buffer with new values.
///
/// # Examples
/// Write-WgpuBuffer $buf ([float[]](1, 2, 3))
/// Write-WgpuBuffer $buf 7 -Offset 100
#[cmdlet(verb = "Write", noun = "WgpuBuffer", alias = ["Write-GpuBuffer"])]
#[derive(Default)]
pub struct WriteWgpuBuffer {
    /// The buffer to write.
    #[param(mandatory, position = 0)]
    pub buffer: Option<PsProxy<Buffer>>,
    /// The values, converted to the buffer's element type.
    #[param(mandatory, position = 1, raw)]
    pub data: Option<PsObject>,
    /// The first element to overwrite; 0 when omitted.
    #[param]
    pub offset: Option<u64>,
}

impl Cmdlet for WriteWgpuBuffer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (Some(buffer), Some(data)) = (&self.buffer, &self.data) else {
            return Err(invalid("WgpuNoData", "a buffer and data are both needed"));
        };
        buffer.with(|b| b.write_data(data, self.offset, Some(ps)))?
    }
}

/// Copies elements from one buffer to another on the GPU.
///
/// Offsets count elements of each buffer's own type; the copy moves the
/// source's bytes, in whole 4-byte words.
///
/// # Examples
/// Copy-WgpuBuffer -Source $a -Destination $b
/// Copy-WgpuBuffer $a $b -SourceOffset 4 -Count 16
#[cmdlet(verb = "Copy", noun = "WgpuBuffer", alias = ["Copy-GpuBuffer"])]
#[derive(Default)]
pub struct CopyWgpuBuffer {
    /// The buffer to copy from.
    #[param(mandatory, position = 0)]
    pub source: Option<PsProxy<Buffer>>,
    /// The buffer to copy to.
    #[param(mandatory, position = 1)]
    pub destination: Option<PsProxy<Buffer>>,
    /// The first source element; 0 when omitted.
    #[param]
    pub source_offset: Option<u64>,
    /// The first destination element; 0 when omitted.
    #[param]
    pub destination_offset: Option<u64>,
    /// The number of source elements; to the end when omitted.
    #[param]
    pub count: Option<u64>,
}

impl Cmdlet for CopyWgpuBuffer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (Some(source), Some(destination)) = (&self.source, &self.destination) else {
            return Err(invalid("WgpuNoBuffer", "a source and a destination are both needed"));
        };
        let (core, raw, r) = source.with(|s| -> PsResult<_> {
            let r = s.copy_range(destination, self.source_offset, self.destination_offset, self.count)?;
            Ok((s.core.clone(), s.buffer.clone(), r))
        })??;
        copy(&core, &raw, r.from, &r.target, r.to, r.size, Some(ps))
    }
}

/// Sets every element of a buffer to zero, or to -Value.
///
/// # Examples
/// Clear-WgpuBuffer $buf
/// $buf | Clear-WgpuBuffer -Value 1.5
#[cmdlet(verb = "Clear", noun = "WgpuBuffer", alias = ["Clear-GpuBuffer"], supports_should_process)]
#[derive(Default)]
pub struct ClearWgpuBuffer {
    /// The buffer to clear.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub buffer: Option<PsProxy<Buffer>>,
    /// The number every element takes, converted to the buffer's element
    /// type as each number of untyped data is; 0 when omitted.
    #[param]
    pub value: Option<PsObject>,
}

impl Cmdlet for ClearWgpuBuffer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(buffer) = &self.buffer else {
            return Err(invalid("WgpuNoBuffer", "no buffer was given"));
        };
        let (core, raw, ty, length, label) = buffer.with(|b| (b.core.clone(), b.buffer.clone(), b.element_type, b.length, b.label.clone()))?;
        let (one, shown) = match &self.value {
            Some(value) => (element::scalar_bytes(ty, value)?, String::from_ps(value)?),
            None => (vec![0; ty.size() as usize], "0".to_string()),
        };
        if !ps.should_process(&label, &format!("Set every element to {shown}"))? {
            return Ok(());
        }
        fill_elements(&core, &raw, &one, length, Some(ps))
    }
}

/// Frees a buffer's GPU memory now, rather than when the object is
/// collected.
///
/// # Examples
/// $buf | Remove-WgpuBuffer
#[cmdlet(verb = "Remove", noun = "WgpuBuffer", alias = ["Remove-GpuBuffer"], supports_should_process)]
#[derive(Default)]
pub struct RemoveWgpuBuffer {
    /// The buffer to free.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub buffer: Option<PsProxy<Buffer>>,
}

impl Cmdlet for RemoveWgpuBuffer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(buffer) = &self.buffer else {
            return Err(invalid("WgpuNoBuffer", "no buffer was given"));
        };
        let label = buffer.with(|b| b.label.clone())?;
        if !ps.should_process(&label, "Free the buffer's GPU memory")? {
            return Ok(());
        }
        buffer.with(|b| b.destroy())?
    }
}
