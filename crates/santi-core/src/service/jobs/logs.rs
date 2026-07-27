use std::{
    fs::File,
    io::{Read as _, Seek, SeekFrom},
};

use super::Service;
use crate::job;

pub struct Read<'a> {
    pub soul: &'a str,
    pub id: &'a str,
    pub stream: job::Stream,
    pub cursor: &'a str,
    pub limit: usize,
}

impl Service {
    pub fn logs(&self, request: Read<'_>) -> Result<Option<job::Log>, String> {
        let Some(record) = self
            .store
            .record(request.id)?
            .filter(|record| record.job.origin.soul == request.soul)
        else {
            return Ok(None);
        };
        let cursor = request
            .cursor
            .parse::<u64>()
            .map_err(|_| "job log cursor is invalid".to_string())?;
        let limit = request.limit.clamp(1, 256 * 1024);
        let filename = match request.stream {
            job::Stream::Stdout => "stdout.log",
            job::Stream::Stderr => "stderr.log",
        };
        let path = self.jobhome(&record).join(filename);
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Some(job::Log {
                    job: request.id.to_string(),
                    stream: request.stream,
                    cursor: cursor.to_string(),
                    next: cursor.to_string(),
                    eof: true,
                    data: String::new(),
                }));
            }
            Err(error) => return Err(error.to_string()),
        };
        let length = file.metadata().map_err(|error| error.to_string())?.len();
        let offset = cursor.min(length);
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        let mut bytes = vec![0; limit];
        let read = file.read(&mut bytes).map_err(|error| error.to_string())?;
        bytes.truncate(read);
        let next = offset.saturating_add(read as u64);
        Ok(Some(job::Log {
            job: request.id.to_string(),
            stream: request.stream,
            cursor: offset.to_string(),
            next: next.to_string(),
            eof: next >= length,
            data: String::from_utf8_lossy(&bytes).into_owned(),
        }))
    }
}
