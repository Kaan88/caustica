//! Vulkan's "Compiling shaders N/22" frame (`render::progress`): cleared rects in a dynamic
//! rendering pass on the swap chain image, so it needs no pipeline.

use super::res::COLOR;
use super::{Device, Frame, Swapchain};
use anyhow::Result;
use ash::vk;

/// Presents one progress frame: `done` of `total` pipelines compiled. Skips the frame when
/// the swap chain is out of date (a resize mid-compile; the first real frame recreates it).
pub unsafe fn draw(
    d: &Device,
    swap: &Swapchain,
    frame: &Frame,
    done: usize,
    total: usize,
) -> Result<()> {
    unsafe {
        let ext = swap.extent;
        if ext.width == 0 || ext.height == 0 {
            return Ok(());
        }
        // Acquire before `begin` resets the fence (once the last frame's wait on the acquire
        // semaphore is done): a failed acquire then leaves the frame fence signalled for the
        // first real frame.
        d.device.wait_for_fences(&[frame.fence], true, u64::MAX)?;
        let index = match swap.fns.acquire_next_image(
            swap.handle,
            u64::MAX,
            frame.acquire,
            vk::Fence::null(),
        ) {
            Ok((i, _)) => Some(i),
            // A resize mid-compile: the first real frame recreates the swap chain.
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        frame.begin(d)?;
        let cmd = frame.cmd;
        if let Some(i) = index {
            let image = swap.images[i as usize];
            let barrier = |old, new| {
                [vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                    .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                    .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)
                    .old_layout(old)
                    .new_layout(new)
                    .image(image)
                    .subresource_range(COLOR)]
            };
            let b = barrier(
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            );
            d.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&b),
            );
            let color = |[r, g, b]: [f32; 3]| vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [r, g, b, 1.],
                },
            };
            let attachments = [vk::RenderingAttachmentInfo::default()
                .image_view(swap.views[i as usize])
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(color(crate::render::progress::BACKGROUND))];
            d.device.cmd_begin_rendering(
                cmd,
                &vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent: ext,
                    })
                    .layer_count(1)
                    .color_attachments(&attachments),
            );
            for (c, rects) in crate::render::progress::layout(ext.width, ext.height, done, total) {
                if rects.is_empty() {
                    continue;
                }
                let rects: Vec<vk::ClearRect> = rects
                    .iter()
                    .map(|&[x, y, w, h]| vk::ClearRect {
                        rect: vk::Rect2D {
                            offset: vk::Offset2D {
                                x: x as i32,
                                y: y as i32,
                            },
                            extent: vk::Extent2D {
                                width: w,
                                height: h,
                            },
                        },
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .collect();
                let a = [vk::ClearAttachment {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    color_attachment: 0,
                    clear_value: color(c),
                }];
                d.device.cmd_clear_attachments(cmd, &a, &rects);
            }
            d.device.cmd_end_rendering(cmd);
            let b = barrier(
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::ImageLayout::PRESENT_SRC_KHR,
            );
            d.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&b),
            );
        }
        d.device.end_command_buffer(cmd)?;
        let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
        let mut submit = vk::SubmitInfo2::default().command_buffer_infos(&cmds);
        let waits = [vk::SemaphoreSubmitInfo::default()
            .semaphore(frame.acquire)
            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
        let signals = index.map(|i| {
            [vk::SemaphoreSubmitInfo::default()
                .semaphore(swap.release[i as usize])
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)]
        });
        if let Some(signals) = &signals {
            submit = submit
                .wait_semaphore_infos(&waits)
                .signal_semaphore_infos(signals);
        }
        d.device.queue_submit2(d.queue, &[submit], frame.fence)?;
        if let Some(i) = index {
            let (waits, chains, indices) = ([swap.release[i as usize]], [swap.handle], [i]);
            // Out of date is fine here: the first real frame recreates the swap chain.
            let _ = swap.fns.queue_present(
                d.queue,
                &vk::PresentInfoKHR::default()
                    .wait_semaphores(&waits)
                    .swapchains(&chains)
                    .image_indices(&indices),
            );
        }
        Ok(())
    }
}
