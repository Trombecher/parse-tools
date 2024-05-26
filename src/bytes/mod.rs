mod tests;

use core::hint::unreachable_unchecked;
use core::marker::PhantomData;
use core::mem::transmute;
use core::ptr::slice_from_raw_parts;
use core::slice::from_raw_parts;

/// An iterator over a slice.
pub struct Cursor<'a> {
    /// The pointer to the first element.
    first: *const u8,
    
    /// The pointer to the next element.
    cursor: *const u8,
    
    /// The pointer to the past-the-end element.
    end: *const u8,
    
    /// The number of bytes consumed.
    index: u64,
    
    /// The marker for ownership of `&[u8]`.
    _marker: PhantomData<&'a [u8]>,
}

/// All errors [crate::bytes] can produce.
#[repr(u8)]
#[derive(Ord, PartialOrd, Eq, PartialEq, Copy, Clone, Debug)]
pub enum Error {
    /// Encountered a continuation byte where the byte 1 was expected.
    EncounteredContinuationByte,
    
    /// The input ended while decoding the second byte of a two byte sequence.
    Missing2ndOf2,
    
    /// The second byte of a two byte sequence is not a continuation byte.
    Invalid2ndOf2,

    /// The input ended while decoding the second byte of a three byte sequence.
    Missing2ndOf3,

    /// The second byte of a three byte sequence is not a continuation byte.
    Invalid2ndOf3,

    /// The input ended while decoding the third byte of a three byte sequence.
    Missing3rdOf3,
    
    /// The third byte of a three byte sequence is not a continuation byte.
    Invalid3rdOf3,
    
    /// The input ended while decoding the second byte of a four byte sequence.
    Missing2ndOf4,
    
    /// The second byte of a four byte sequence is not a continuation byte.
    Invalid2ndOf4,

    /// The input ended while decoding the third byte of a four byte sequence.
    Missing3rdOf4,
    
    /// The third byte of a four byte sequence is not a continuation byte.
    Invalid3rdOf4,

    /// The input ended while decoding the fourth byte of a four byte sequence.
    Missing4thOf4,
    
    /// The fourth byte of a four byte sequence is not a continuation byte.
    Invalid4thOf4,
}

impl<'a> Cursor<'a> {
    #[inline]
    pub const fn as_slice(&self) -> &[u8] {
        unsafe { from_raw_parts(self.first, self.end.offset_from(self.first) as usize) }
    }
    
    #[inline]
    pub const fn cursor(&self) -> *const u8 {
        self.cursor
    }
    
    #[inline]
    pub const fn new(slice: &[u8]) -> Self {
        Self {
            first: slice.as_ptr(),
            cursor: slice.as_ptr(),
            end: unsafe { slice.as_ptr().add(slice.len()) },
            index: 0,
            _marker: PhantomData,
        }
    }
    
    /// Gets the next byte. Normalizes line terminators by mapping CR, CRLF and LF sequences to LF.
    #[inline]
    pub fn next_lfn(&mut self) -> Option<u8> {
        match self.peek() {
            None => None,
            Some(b'\r') => {
                unsafe { self.advance_unchecked() }
                
                if self.peek() == Some(b'\n') {
                    // SAFETY: Because of `Some(...)` there is a next byte.
                    self.cursor = unsafe { self.cursor.add(1) };
                }
                Some(b'\n')
            }
            x => {
                unsafe { self.advance_unchecked() }
                x
            }
        }
    }
    
    /// Gets the next byte. Does not normalize line terminators.
    #[inline]
    pub fn next(&mut self) -> Option<u8> {
        if !self.has_next() {
            None
        } else {
            let byte = unsafe { self.peek_unchecked() };
            unsafe { self.advance_unchecked() };
            Some(byte)
        }
    }

    /// Gets the next byte. Does not normalize line terminators.
    /// 
    /// # Safety
    /// 
    /// The caller must ensure that the cursor has a next byte.
    #[inline]
    pub unsafe fn next_unchecked(&mut self) -> u8 {
        let byte = self.peek_unchecked();
        self.advance_unchecked();
        byte
    }

    /// Peeks into the next byte. Does not advance the iterator.
    #[inline]
    pub fn peek(&self) -> Option<u8> {
        if !self.has_next() {
            None
        } else {
            Some(unsafe { self.peek_unchecked() })
        }
    }
    
    /// Checks if the cursor has a next byte.
    #[inline]
    pub fn has_next(&self) -> bool {
        self.cursor < self.end
    }
    
    /// Peeks into the next byte. Does not advance the iterator.
    /// 
    /// # Safety
    /// 
    /// The caller must ensure that the cursor has a next byte.
    #[inline]
    pub unsafe fn peek_unchecked(&self) -> u8 {
        *self.cursor
    }

    #[inline]
    pub fn rewind_lfn(&mut self) {
        if self.can_rewind() {
            self.index -= 1;
            self.cursor = unsafe { self.cursor.sub(1) };
            
            if unsafe { *self.cursor } == b'\n'
                && self.cursor != self.first
                && unsafe { *self.cursor.sub(1) } == b'\r' {
                self.cursor = unsafe { self.cursor.sub(1) };
            }
        }
    }
    
    /// Checks if the cursor can be rewinded.
    #[inline]
    pub fn can_rewind(&mut self) -> bool {
        self.cursor > self.first
    }
    
    /// Rewinds one byte. Saturates at the lower boundary.
    #[inline]
    pub fn rewind(&mut self) {
        if self.can_rewind() {
            unsafe { self.rewind_unchecked(); }
        }
    }
    
    /// Rewinds one byte.
    /// 
    /// # Safety
    /// 
    /// The caller must ensure that the cursor can rewind.
    #[inline]
    pub unsafe fn rewind_unchecked(&mut self) {
        self.index -= 1;
        self.cursor = self.cursor.sub(1);
    }
    
    /// Advances one char, saturates at the upper boundary.
    #[inline]
    pub fn advance(&mut self) {
        if self.has_next() {
            unsafe { self.advance_unchecked(); }
        }
    }
    
    /// Advances the cursor one byte.
    /// 
    /// # Safety
    /// 
    /// The caller must ensure that the cursor is not at the end.
    #[inline]
    pub unsafe fn advance_unchecked(&mut self) {
        self.index += 1;
        self.cursor = self.cursor.add(1)
    }

    #[inline]
    pub unsafe fn advance_char_unchecked(&mut self) {
        self.index += 1;
        self.cursor = self.cursor.add(UTF8_CHAR_WIDTH[self.peek_unchecked() as usize] as usize);
    }
    
    /// Advances the cursor by one char encoded as UTF-8.
    #[inline]
    pub fn advance_char(&mut self) -> Result<(), Error> {
        self.index += 1;
        
        let first_byte = match self.next() {
            Some(x) => x,
            None => return Ok(()),
        };

        macro_rules! next {
            ($e:expr,$i:expr) => {
                match self.next_lfn() {
                    None => return Err($e),
                    Some(x) if x & 0b1100_0000 != 0b1000_0000 => return Err($i),
                    _ => {},
                }
            };
        }

        match UTF8_CHAR_WIDTH[first_byte as usize] {
            0 => Err(Error::EncounteredContinuationByte),
            1 => {
                if first_byte == b'\r' && self.peek() == Some(b'\n')  {
                    unsafe { self.advance_unchecked() }
                }
                Ok(())
            },
            2 => {
                next!(Error::Missing2ndOf2, Error::Invalid2ndOf2);
                Ok(())
            }
            3 => {
                next!(Error::Missing2ndOf3, Error::Invalid2ndOf3);
                next!(Error::Missing3rdOf3, Error::Invalid3rdOf3);
                Ok(())
            }
            4 => {
                next!(Error::Missing2ndOf4, Error::Invalid2ndOf4);
                next!(Error::Missing3rdOf4, Error::Invalid3rdOf4);
                next!(Error::Missing4thOf4, Error::Invalid4thOf4);
                Ok(())
            }
            _ => unsafe { unreachable_unchecked() }
        }
    }

    #[inline]
    pub fn begin_recording<'c>(&'c mut self) -> Recorder<'a, 'c> {
        Recorder {
            start: self.cursor,
            cursor: self,
        }
    }
    
    #[inline]
    pub const fn index(&self) -> u64 {
        self.index
    }
}

pub struct Recorder<'a, 'c> {
    pub cursor: &'c mut Cursor<'a>,
    start: *const u8,
}

impl<'a, 'c> Recorder<'a, 'c> {
    #[inline]
    pub fn stop(self) -> &'a str {
        unsafe { transmute(slice_from_raw_parts(
            self.start,
            self.start.offset_from(self.cursor.cursor).unsigned_abs()
        )) }
    }
}

const UTF8_CHAR_WIDTH: &[u8; 256] = &[
    // 1  2  3  4  5  6  7  8  9  A  B  C  D  E  F
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 0
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 1
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 2
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 3
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 4
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 5
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 6
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 7
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 8
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 9
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // A
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // B
    0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // C
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // D
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, // E
    4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // F
];