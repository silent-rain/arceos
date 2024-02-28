//! 提供了一个双向合并的内存分配器实现。
//! 这个实现基于链表，当释放内存时，会检查前后相邻的空闲内存块，并将它们合并成一个更大的空闲内存块。
//! 这可以减少内存碎片。

extern crate alloc;

use super::{AllocError, AllocResult, BaseAllocator, ByteAllocator};
use core::alloc::Layout;
use core::ptr::NonNull;

/// 双向合并内存分配器
pub struct MergingAllocator {
    // 链表头指针
    head: Option<NonNull<FreeBlock>>,
    // 已分配字节数
    total_bytes: usize,
    // 已使用字节数
    used_bytes: usize,
}

/// 空闲内存块
struct FreeBlock {
    // 块的大小
    size: usize,
    // 指向下一个空闲内存块的指针
    next: Option<NonNull<FreeBlock>>,
}

impl MergingAllocator {
    /// 创建一个新的实例
    pub const fn new() -> Self {
        Self {
            head: None,
            total_bytes: 0,
            used_bytes: 0,
        }
    }

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

    fn request_memory(&mut self, size: usize) -> Result<(usize, usize), AllocError> {
        // const MIN_SIZE: usize = 4096;
        // 计算实际请求的内存大小，取 size 和 MIN_SIZE 的较大值，这样可以避免频繁地请求内存
        // let request_size = size.max(MIN_SIZE);
        assert!(size > 0, "size must be positive"); // 检查 size 是否为正
        let layout = Layout::from_size_align(size, 1 << 2).unwrap(); // 创建 Layout
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) }; // 分配内存
        if ptr.is_null() {
            return Err(AllocError::NoMemory); // 分配失败
        }
        let start = ptr as usize;
        // unsafe { alloc::alloc::dealloc(ptr, layout) };
        Ok((start, size))
    }
}

impl BaseAllocator for MergingAllocator {
    /// 初始化内存分配器
    fn init(&mut self, start: usize, size: usize) {
        let mut block = NonNull::new(start as *mut FreeBlock).unwrap();
        unsafe { block.as_mut().size = size };
        self.insert_free_block(block);
        self.total_bytes = size;
    }

    /// 向内存分配器添加内存
    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        let mut block = NonNull::new(start as *mut FreeBlock).unwrap();
        unsafe { block.as_mut().size = size };
        self.insert_free_block(block);
        self.total_bytes += size;
        Ok(())
    }
}

impl ByteAllocator for MergingAllocator {
    /// 分配内存
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        // 计算分配内存的大小，取较大值。
        let size = layout.size().max(layout.align());
        // 查找一个足够大的空闲内存块
        let mut block = match self.find_free_block(size) {
            Some(block) => block,
            None => {
                // 如果找不到空闲的内存块，尝试添加一个新的内存块
                // 从系统或其他来源获取一块内存
                let (start, size) = self.request_memory(size)?;
                // 将新的内存块添加到分配器中
                self.add_memory(start, size)?;
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
