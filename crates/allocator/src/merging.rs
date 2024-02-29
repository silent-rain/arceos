//! 提供了一个双向合并的内存分配器实现。
//! 这个实现基于链表，当释放内存时，会检查前后相邻的空闲内存块，并将它们合并成一个更大的空闲内存块。
//! 这可以减少内存碎片。
extern crate alloc;

extern crate spin;

use super::{AllocError, AllocResult, BaseAllocator, ByteAllocator};
use crate::{BitmapPageAllocator, PageAllocator};
use core::alloc::Layout;
use core::ptr::NonNull;
use spin::Mutex;
use spinlock::SpinNoIrq;

const PAGE_SIZE: usize = 0x1000;

/// 双向合并内存分配器
pub struct MergingAllocator {
    // 使用 Mutex 包装链表头指针和统计数据，以确保线程安全
    inner: Mutex<MergingAllocatorInner>,
}

/// MergingAllocator 的内部状态，现在被 Mutex 保护
struct MergingAllocatorInner {
    // 链表头指针
    head: Option<NonNull<FreeBlock>>,
    // 已分配字节数
    total_bytes: usize,
    // 已使用字节数
    used_bytes: usize,
    palloc: SpinNoIrq<BitmapPageAllocator<PAGE_SIZE>>,
}

/// 空闲内存块
struct FreeBlock {
    // 块的大小
    size: usize,
    // 指向下一个空闲内存块的指针
    next: Option<NonNull<FreeBlock>>,
}

unsafe impl Send for FreeBlock {}
unsafe impl Sync for FreeBlock {}

impl MergingAllocatorInner {
    /// 查找大小至少为size的空闲内存块
    fn find_free_block(&mut self, size: usize) -> Option<NonNull<FreeBlock>> {
        let mut current = &mut self.head;

        while let Some(mut block) = *current {
            // 检查当前内存块的大小是否大于等于所需的大小。
            // 如果当前内存块的大小足够，返回一个包含当前内存块的Some值。
            if unsafe { block.as_ref().size } >= size {
                return Some(block);
            }
            current = unsafe { &mut block.as_mut().next };
        }

        None
    }

    /// 将空闲内存块插入到链表中
    fn insert_free_block(&mut self, mut block: NonNull<FreeBlock>) {
        let mut current = &mut self.head;

        while let Some(mut next_block) = *current {
            // 如果要插入的内存块的指针小于当前内存块的指针，跳出循环。
            if block.as_ptr() < next_block.as_ptr() {
                break;
            }
            current = unsafe { &mut next_block.as_mut().next };
        }

        unsafe { block.as_mut().next = *current };
        *current = Some(block);
    }

    /// 从链表中移除空闲内存块
    fn remove_free_block(&mut self, block: NonNull<FreeBlock>) {
        let mut current = &mut self.head;

        while let Some(mut next_block) = *current {
            // 检查next_block是否等于要移除的block。
            if next_block == block {
                *current = unsafe { next_block.as_ref().next };
                break;
            }
            current = unsafe { &mut next_block.as_mut().next };
        }
    }

    /// 合并相邻的空闲内存块
    fn merge_adjacent_blocks(&mut self, mut block: NonNull<FreeBlock>) {
        let mut current = &mut self.head;

        while let Some(mut next_block) = *current {
            if next_block.as_ptr() as usize + unsafe { next_block.as_ref().size }
                == block.as_ptr() as usize
            {
                // 检查next_block的地址加上其大小是否等于block的地址。
                unsafe { block.as_mut().size += next_block.as_ref().size };
                self.remove_free_block(next_block);
            } else if block.as_ptr() as usize + unsafe { block.as_ref().size }
                == next_block.as_ptr() as usize
            {
                // 检查block的地址加上其大小是否等于next_block的地址。
                unsafe { next_block.as_mut().size += block.as_ref().size };
                self.remove_free_block(block);
                break;
            }

            // 如果block和next_block都不满足合并条件，将current更新为指向下一个内存块的可变引用。
            current = unsafe { &mut next_block.as_mut().next };
        }
    }

    /// 检查新添加的内存区域是否与现有的内存区域重叠
    fn checked_block(&mut self, start: usize, size: usize) -> AllocResult<()> {
        let new_end = start.checked_add(size).ok_or(AllocError::InvalidParam)?;

        let mut current = self.head;

        while let Some(block) = current {
            let block_ptr = block.as_ptr() as usize;
            let block_end = block_ptr
                .checked_add(unsafe { block.as_ref().size })
                .ok_or(AllocError::InvalidParam)?;

            if start < block_end && new_end > block_ptr {
                // 新内存区域与现有内存区域重叠
                return Err(AllocError::MemoryOverlap);
            }
            current = unsafe { block.as_ref().next };
        }

        Ok(())
    }
}

unsafe impl Send for MergingAllocatorInner {}
unsafe impl Sync for MergingAllocatorInner {}

impl BaseAllocator for MergingAllocatorInner {
    /// 初始化内存分配器
    fn init(&mut self, start: usize, size: usize) {
        let mut block = NonNull::new(start as *mut FreeBlock).unwrap();
        unsafe { block.as_mut().size = size };
        self.insert_free_block(block);
        self.total_bytes = size;
    }

    /// 向内存分配器添加内存
    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        self.checked_block(start, size)?;

        let mut block = NonNull::new(start as *mut FreeBlock).unwrap();
        unsafe { block.as_mut().size = size };
        self.insert_free_block(block);
        self.total_bytes += size;
        Ok(())
    }
}

impl ByteAllocator for MergingAllocatorInner {
    /// 分配内存
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        // 计算分配内存的大小，取较大值。
        let size = layout.size().max(layout.align());
        // 查找一个足够大的空闲内存块
        let mut block = match self.find_free_block(size) {
            Some(block) => block,
            None => {
                // 如果找不到空闲的内存块，尝试添加一个新的内存块
                let old_size = self.total_bytes();
                let expand_size = old_size
                    .max(layout.size())
                    .next_power_of_two()
                    .max(PAGE_SIZE);

                let heap_ptr = self
                    .palloc
                    .lock()
                    .alloc_pages(expand_size / PAGE_SIZE, PAGE_SIZE)?;
                // 将新的内存块添加到分配器中
                self.add_memory(heap_ptr, size)?;
                // 再次查找空闲的内存块，这次应该能找到
                self.find_free_block(size).ok_or(AllocError::NoMemory)?
            }
        };
        // 空闲内存块大小
        let block_size = unsafe { block.as_ref().size };

        // 确保分配的内存块满足内存对齐要求，同时尽可能地利用空闲内存块
        // 计算对齐后的指针
        let aligned_ptr =
            ((block.as_ptr() as usize + layout.align() - 1) & !(layout.align() - 1)) as *mut u8;
        // 计算对齐后的内存块大小
        let aligned_size = block_size - (aligned_ptr as usize - block.as_ptr() as usize);

        // 当计算对齐后的内存块大小大于等于找到的块的大小时，
        // 将找到的块大小重置为对齐后的大小，然后移除该空闲块。
        if aligned_size >= size {
            unsafe { block.as_mut().size = aligned_size };
            self.remove_free_block(block);
        }

        self.used_bytes += aligned_size;
        Ok(NonNull::new(aligned_ptr).unwrap())
    }

    /// 释放内存
    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout) {
        let size = layout.size().max(layout.align());

        let mut block = NonNull::new(pos.as_ptr() as *mut FreeBlock).unwrap();
        unsafe { block.as_mut().size = size };

        self.insert_free_block(block);
        self.merge_adjacent_blocks(block);
        self.used_bytes -= size;
    }

    /// 返回总字节数
    fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// 返回已使用字节数
    fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// 返回可用字节数
    fn available_bytes(&self) -> usize {
        self.total_bytes - self.used_bytes
    }
}

impl MergingAllocator {
    /// 创建一个新的实例
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(MergingAllocatorInner {
                head: None,
                total_bytes: 0,
                used_bytes: 0,
                palloc: SpinNoIrq::new(BitmapPageAllocator::new()),
            }),
        }
    }
}

impl BaseAllocator for MergingAllocator {
    /// 初始化内存分配器
    fn init(&mut self, start: usize, size: usize) {
        let mut inner = self.inner.lock();
        inner.init(start, size);
    }

    /// 向内存分配器添加内存
    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        let mut inner = self.inner.lock();
        inner.add_memory(start, size)
    }
}

impl ByteAllocator for MergingAllocator {
    /// 分配内存
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        let mut inner = self.inner.lock();
        inner.alloc(layout)
    }

    /// 释放内存
    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout) {
        let mut inner = self.inner.lock();
        inner.dealloc(pos, layout);
    }

    /// 返回总字节数
    fn total_bytes(&self) -> usize {
        let inner = self.inner.lock();
        inner.total_bytes()
    }

    /// 返回已使用字节数
    fn used_bytes(&self) -> usize {
        let inner = self.inner.lock();
        inner.used_bytes()
    }

    /// 返回可用字节数
    fn available_bytes(&self) -> usize {
        let inner = self.inner.lock();
        inner.available_bytes()
    }
}
