use super::{
    Result, VideoError,
    capture::{CapturedFrame, PixelFormat},
};
use fast_image_resize::{PixelType, Resizer, images::Image};
use openh264::formats::YUVSource;
use yuvutils_rs::{
    BufferStoreMut, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix,
};

/// BT.709 limited-range I420, including chroma. Encoders advertise the same VUI.
#[derive(Clone, Debug)]
pub struct I420Frame {
    pub width: u32,
    pub height: u32,
    data: Vec<u8>,
}
impl I420Frame {
    pub fn from_capture(frame: &CapturedFrame) -> Result<Self> {
        frame.validate()?;
        if !frame.width.is_multiple_of(2) || !frame.height.is_multiple_of(2) {
            // The conversion/scaling path crops one right/bottom pixel for 4:2:0.
            let width = frame.width & !1;
            let height = frame.height & !1;
            if width == 0 || height == 0 {
                return Err(VideoError::Invalid("I420 needs at least 2x2"));
            }
            let mut cropped = frame.clone();
            cropped.width = width;
            cropped.height = height;
            return Self::from_capture(&cropped);
        }
        let len = frame.width as usize * frame.height as usize;
        let mut data = vec![0; len * 3 / 2];
        let (y, uv) = data.split_at_mut(len);
        let (u, v) = uv.split_at_mut(len / 4);
        let mut image = YuvPlanarImageMut {
            y_plane: BufferStoreMut::Borrowed(y),
            y_stride: frame.width,
            u_plane: BufferStoreMut::Borrowed(u),
            u_stride: frame.width / 2,
            v_plane: BufferStoreMut::Borrowed(v),
            v_stride: frame.width / 2,
            width: frame.width,
            height: frame.height,
        };
        let convert = match frame.format {
            PixelFormat::Bgrx => yuvutils_rs::bgra_to_yuv420,
            PixelFormat::Rgbx => yuvutils_rs::rgba_to_yuv420,
        };
        convert(
            &mut image,
            &frame.data,
            u32::try_from(frame.stride).map_err(|_| VideoError::Invalid("stride too large"))?,
            YuvRange::Limited,
            YuvStandardMatrix::Bt709,
            YuvConversionMode::Balanced,
        )
        .map_err(|e| VideoError::Encoder(e.to_string()))?;
        Ok(Self {
            width: frame.width,
            height: frame.height,
            data,
        })
    }
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn scale(&self, width: u32, height: u32) -> Result<Self> {
        if width < 2
            || height < 2
            || width > 4096
            || height > 2160
            || !width.is_multiple_of(2)
            || !height.is_multiple_of(2)
        {
            return Err(VideoError::Invalid(
                "I420 scaled size must be even and fit 4096x2160",
            ));
        }
        if (width, height) == (self.width, self.height) {
            return Ok(self.clone());
        }
        let mut data = Vec::with_capacity(width as usize * height as usize * 3 / 2);
        let mut resizer = Resizer::new();
        for (plane, sw, sh, dw, dh) in [
            (self.y(), self.width, self.height, width, height),
            (
                self.u(),
                self.width / 2,
                self.height / 2,
                width / 2,
                height / 2,
            ),
            (
                self.v(),
                self.width / 2,
                self.height / 2,
                width / 2,
                height / 2,
            ),
        ] {
            let mut plane = plane.to_vec();
            let src = Image::from_slice_u8(sw, sh, &mut plane, PixelType::U8)
                .map_err(|e| VideoError::Encoder(e.to_string()))?;
            let mut dst = Image::new(dw, dh, PixelType::U8);
            resizer
                .resize(&src, &mut dst, None)
                .map_err(|e| VideoError::Encoder(e.to_string()))?;
            data.extend_from_slice(dst.buffer());
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }
    pub fn nv12(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len());
        out.extend_from_slice(self.y());
        for (u, v) in self.u().iter().zip(self.v()) {
            out.extend_from_slice(&[*u, *v]);
        }
        out
    }
}
impl YUVSource for I420Frame {
    fn dimensions(&self) -> (usize, usize) {
        (self.width as usize, self.height as usize)
    }
    fn dimensions_i32(&self) -> (i32, i32) {
        (self.width as i32, self.height as i32)
    }
    fn strides(&self) -> (usize, usize, usize) {
        (
            self.width as usize,
            self.width as usize / 2,
            self.width as usize / 2,
        )
    }
    fn y(&self) -> &[u8] {
        &self.data[..self.width as usize * self.height as usize]
    }
    fn u(&self) -> &[u8] {
        let n = self.width as usize * self.height as usize;
        &self.data[n..n * 5 / 4]
    }
    fn v(&self) -> &[u8] {
        let n = self.width as usize * self.height as usize;
        &self.data[n * 5 / 4..]
    }
}
/// Full YUV PSNR, compared plane by plane to account for decoder row padding.
pub fn psnr(reference: &impl YUVSource, decoded: &impl YUVSource) -> Result<f64> {
    if reference.dimensions() != decoded.dimensions() {
        return Err(VideoError::Invalid("PSNR dimensions differ"));
    }
    let (w, h) = reference.dimensions();
    let rs = reference.strides();
    let ds = decoded.strides();
    let mut error = 0.0;
    let mut count = 0usize;
    for (r, d, width, height, rstride, dstride) in [
        (reference.y(), decoded.y(), w, h, rs.0, ds.0),
        (reference.u(), decoded.u(), w / 2, h / 2, rs.1, ds.1),
        (reference.v(), decoded.v(), w / 2, h / 2, rs.2, ds.2),
    ] {
        for row in 0..height {
            for col in 0..width {
                error += (r[row * rstride + col] as f64 - d[row * dstride + col] as f64).powi(2);
                count += 1;
            }
        }
    }
    Ok(if error == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64.powi(2) * count as f64 / error).log10()
    })
}
