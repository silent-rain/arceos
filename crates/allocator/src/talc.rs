use super::{AllocError, AllocResult, BaseAllocator, ByteAllocator};
use core::alloc::{Allocator, Layout};
use core::ptr::NonNull;
use talc::{ErrOnOom, Talc, Talck};

// 基于 talc 的内存分配器
pub struct TalcByteAllocator {
    // 使用 spin::Mutex 作为锁类型，使用 ErrOnOom 作为内存不足处理器
    inner: Talck<spin::Mutex<()>, ErrOnOom>,
    total_bytes: usize,
    used_bytes: usize,
}

impl TalcByteAllocator {
    /// 创建一个新的空的内存分配器
    pub const fn new() -> Self {
        let talck = Talc::new(ErrOnOom).lock::<spin::Mutex<()>>();

        Self {
            inner: talck,
            total_bytes: 0,
            used_bytes: 0,
        }
    }
}

impl BaseAllocator for TalcByteAllocator {
    fn init(&mut self, start: usize, size: usize) {
        unsafe {
            let pool = core::slice::from_raw_parts_mut(start as *mut u8, size);
            // 使用 Talck::claim 来添加内存区域
            let _ = self.inner.lock().claim(pool.into());
        }
        self.total_bytes = size;
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        unsafe {
            let pool = core::slice::from_raw_parts_mut(start as *mut u8, size);
            // 使用 Talck::claim 来添加内存区域
            let _ = self.inner.lock().claim(pool.into());
        }
        self.total_bytes += size;
        Ok(())
    }
}

impl ByteAllocator for TalcByteAllocator {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        let ptr = self
            .inner
            .allocate(layout)
            // 使用 Talck::allocate 来分配内存，返回一个 Result<NonNull<[u8]>, AllocError>
            .map_err(|_e| AllocError::NoMemory)?;
        // 使用 NonNull::cast 来将 NonNull<[u8]> 转换为 NonNull<u8>
        let ptr = NonNull::cast(ptr);
        self.used_bytes += layout.size();
        Ok(ptr)
    }

    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout) {
        // 使用 Talck::deallocate 来释放内存
        unsafe { self.inner.deallocate(pos, layout) }
        self.used_bytes -= layout.size();
    }

    fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    fn available_bytes(&self) -> usize {
        self.total_bytes - self.used_bytes
    }
}
