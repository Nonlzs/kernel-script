use super::ProtocolError;

/// Message identifiers are part of the stable Named Pipe wire protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum MessageType {
    FetchProcessList = 1,
    ProcessList = 2,
    ReadProcessMemory = 3,
    ReadProcessMemoryResponse = 4,
    WriteProcessMemory = 5,
    WriteProcessMemoryResponse = 6,
    GetProcessBase = 9,
    GetProcessBaseResponse = 10,
    ReadMemoryRva = 11,
    WriteMemoryRva = 12,
    ReadMemoryMdl = 15,
    WriteMemoryMdl = 16,
    ReadMemoryMdlRva = 17,
    WriteMemoryMdlRva = 18,
    GetProcessId = 7,
    GetProcessIdResponse = 8,
    BatchReadMemory = 19,
    BatchReadMemoryResponse = 20,
    TraversePointerChain = 21,
    TraversePointerChainResponse = 22,
    LockMemory = 23,
    UnlockMemory = 24,
    ClearMemoryLocks = 25,
    LockMemoryResponse = 26,
    LockMemoryRva = 27,
    UnlockMemoryRva = 28,
    BatchWriteMemory = 29,
    BatchWriteMemoryResponse = 30,
    Error = 0xFFFF,
    ErrorDetail = 0xFFFE,
}

impl MessageType {
    /// Decode the numeric ID without accepting unknown protocol extensions.
    pub fn from_u16(value: u16) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::FetchProcessList),
            2 => Ok(Self::ProcessList),
            3 => Ok(Self::ReadProcessMemory),
            4 => Ok(Self::ReadProcessMemoryResponse),
            5 => Ok(Self::WriteProcessMemory),
            6 => Ok(Self::WriteProcessMemoryResponse),
            9 => Ok(Self::GetProcessBase),
            10 => Ok(Self::GetProcessBaseResponse),
            11 => Ok(Self::ReadMemoryRva),
            12 => Ok(Self::WriteMemoryRva),
            15 => Ok(Self::ReadMemoryMdl),
            16 => Ok(Self::WriteMemoryMdl),
            17 => Ok(Self::ReadMemoryMdlRva),
            18 => Ok(Self::WriteMemoryMdlRva),
            7 => Ok(Self::GetProcessId),
            8 => Ok(Self::GetProcessIdResponse),
            19 => Ok(Self::BatchReadMemory),
            20 => Ok(Self::BatchReadMemoryResponse),
            21 => Ok(Self::TraversePointerChain),
            22 => Ok(Self::TraversePointerChainResponse),
            23 => Ok(Self::LockMemory),
            24 => Ok(Self::UnlockMemory),
            25 => Ok(Self::ClearMemoryLocks),
            26 => Ok(Self::LockMemoryResponse),
            27 => Ok(Self::LockMemoryRva),
            28 => Ok(Self::UnlockMemoryRva),
            29 => Ok(Self::BatchWriteMemory),
            30 => Ok(Self::BatchWriteMemoryResponse),
            0xFFFF => Ok(Self::Error),
            0xFFFE => Ok(Self::ErrorDetail),
            _ => Err(ProtocolError::UnknownMessageType),
        }
    }
}
