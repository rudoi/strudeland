//! Long live reverbs share each input FFT across the late IR partitions.

use std::sync::Arc;

use super::{FftFrame, FftKernel, MAX_FFT_SIZE};
use crate::QUANTUM;

const HOP: usize = MAX_FFT_SIZE / 2;
const QUANTA_PER_HOP: usize = HOP / QUANTUM;
/// Reserve two hops for the early convolver. After an input block completes, the tail
/// has one full hop to compute the block coming due.
pub(super) const START: usize = 2 * HOP;

#[derive(Debug, Clone)]
pub(super) struct Tail {
    kernels: Arc<[FftKernel]>,
    frame: FftFrame,
    history: Vec<f32>,
    next_spectrum: usize,
    valid_blocks: usize,
    position: usize,
    input: Vec<f32>,
    output: Vec<f32>,
    overlap: Vec<f32>,
}

impl Tail {
    pub(super) fn new(ir: &[f32], scale: f32) -> Self {
        let mut frame = FftFrame::new(MAX_FFT_SIZE).expect("tail FFT size is a power of two");
        let mut input = vec![0.0; MAX_FFT_SIZE];
        let kernels = ir
            .chunks(HOP)
            .map(|partition| {
                input[..partition.len()].copy_from_slice(partition);
                input[partition.len()..HOP].fill(0.0);
                frame.do_fft(&input);
                FftKernel::from_frame(&frame, scale)
            })
            .collect();
        Self::fresh(kernels, frame)
    }

    fn fresh(kernels: Arc<[FftKernel]>, frame: FftFrame) -> Self {
        Self {
            history: vec![0.0; kernels.len() * MAX_FFT_SIZE],
            kernels,
            frame,
            next_spectrum: 0,
            valid_blocks: 0,
            position: 0,
            input: vec![0.0; MAX_FFT_SIZE],
            output: vec![0.0; MAX_FFT_SIZE],
            overlap: vec![0.0; HOP],
        }
    }

    /// Instantiate a cached IR without copying or reading its empty spectrum history.
    pub(super) fn fresh_history(&self) -> Self {
        Self::fresh(self.kernels.clone(), self.frame.clone())
    }

    pub(super) fn process(&mut self, source: &[f32; QUANTUM], dest: &mut [f32; QUANTUM]) {
        let end = self.position + QUANTUM;
        self.input[self.position..end].copy_from_slice(source);
        for (sample, &tail) in dest.iter_mut().zip(&self.output[self.position..end]) {
            *sample += tail;
        }

        let quantum = self.position / QUANTUM;
        let start_partition = quantum * self.valid_blocks / QUANTA_PER_HOP;
        let end_partition = (quantum + 1) * self.valid_blocks / QUANTA_PER_HOP;
        let count = self.kernels.len();
        for partition in start_partition..end_partition {
            // The latest spectrum pairs with kernel zero, the preceding one with kernel one.
            let index = (self.next_spectrum + count - 1 - partition) % count;
            let offset = index * MAX_FFT_SIZE;
            let (real, imag) = self.history[offset..offset + MAX_FFT_SIZE].split_at(HOP);
            accumulate_spectrum(&mut self.frame, &self.kernels[partition], real, imag);
        }

        self.position = end;
        if end == HOP {
            // Finish the preceding input block before overwriting the oldest spectrum.
            if self.valid_blocks > 0 {
                self.frame.do_inverse_fft(&mut self.output);
                for (sample, &overlap) in self.output[..HOP].iter_mut().zip(&self.overlap) {
                    *sample += overlap;
                }
                self.overlap.copy_from_slice(&self.output[HOP..]);
            }

            self.frame.do_fft(&self.input);
            let offset = self.next_spectrum * MAX_FFT_SIZE;
            let (real, imag) = self.history[offset..offset + MAX_FFT_SIZE].split_at_mut(HOP);
            real.copy_from_slice(&self.frame.real);
            imag.copy_from_slice(&self.frame.imag);
            self.next_spectrum = (self.next_spectrum + 1) % count;
            self.valid_blocks = (self.valid_blocks + 1).min(count);
            self.frame.real.fill(0.0);
            self.frame.imag.fill(0.0);
            self.position = 0;
        }
    }
}

fn accumulate_spectrum(frame: &mut FftFrame, kernel: &FftKernel, real: &[f32], imag: &[f32]) {
    // The packed DC and Nyquist bins are two independent real products.
    frame.real[0] += real[0] * kernel.real[0];
    frame.imag[0] += imag[0] * kernel.imag[0];
    for (((sum_re, sum_im), (&x_re, &x_im)), (&h_re, &h_im)) in frame.real[1..]
        .iter_mut()
        .zip(&mut frame.imag[1..])
        .zip(real[1..].iter().zip(&imag[1..]))
        .zip(kernel.real[1..].iter().zip(&kernel.imag[1..]))
    {
        *sum_re += x_re * h_re - x_im * h_im;
        *sum_im += x_re * h_im + x_im * h_re;
    }
}
