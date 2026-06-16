use std::io::{self, Read, Write};

pub struct IoRedirect {
    pub stdin: Vec<u8>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl IoRedirect {
    pub fn new() -> Self {
        IoRedirect {
            stdin: Vec::new(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    pub fn with_stdin(input: &str) -> Self {
        IoRedirect {
            stdin: input.as_bytes().to_vec(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    pub fn write_stdout(&mut self, data: &str) {
        self.stdout.extend_from_slice(data.as_bytes());
    }

    pub fn write_stderr(&mut self, data: &str) {
        self.stderr.extend_from_slice(data.as_bytes());
    }

    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }

    pub fn stdin_str(&self) -> String {
        String::from_utf8_lossy(&self.stdin).to_string()
    }
}

pub fn capture_piped_output<F>(f: F) -> (String, String)
where
    F: FnOnce(&mut IoRedirect),
{
    let mut io = IoRedirect::new();
    f(&mut io);
    (io.stdout_str(), io.stderr_str())
}

pub struct StdioCapture {
    stdout: Vec<u8>,
    #[allow(dead_code)]
    stderr: Vec<u8>,
}

impl StdioCapture {
    pub fn new() -> Self {
        StdioCapture {
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }
}

impl Write for StdioCapture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stdout.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PipeBuffer {
    data: Vec<u8>,
    pos: usize,
}

impl PipeBuffer {
    pub fn new() -> Self {
        PipeBuffer {
            data: Vec::new(),
            pos: 0,
        }
    }

    pub fn from_str(s: &str) -> Self {
        PipeBuffer {
            data: s.as_bytes().to_vec(),
            pos: 0,
        }
    }

    pub fn into_string(self) -> String {
        String::from_utf8_lossy(&self.data).to_string()
    }
}

impl Write for PipeBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.data.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for PipeBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.data.len() {
            return Ok(0);
        }
        let remaining = self.data.len() - self.pos;
        let to_read = buf.len().min(remaining);
        buf[..to_read].copy_from_slice(&self.data[self.pos..self.pos + to_read]);
        self.pos += to_read;
        Ok(to_read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_redirect() {
        let mut io = IoRedirect::with_stdin("input data");
        io.write_stdout("output data");
        io.write_stderr("error data");

        assert_eq!(io.stdin_str(), "input data");
        assert_eq!(io.stdout_str(), "output data");
        assert_eq!(io.stderr_str(), "error data");
    }

    #[test]
    fn test_capture_piped_output() {
        let (out, err) = capture_piped_output(|io| {
            io.write_stdout("hello");
            io.write_stderr("world");
        });
        assert_eq!(out, "hello");
        assert_eq!(err, "world");
    }

    #[test]
    fn test_pipe_buffer() {
        let mut buffer = PipeBuffer::from_str("hello world");
        let mut out = [0u8; 5];
        let n = buffer.read(&mut out).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&out, b"hello");
    }
}
