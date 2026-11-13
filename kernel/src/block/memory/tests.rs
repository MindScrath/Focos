//! 内存盘的契约接入与专属断言：薄壳用内存盘跑共享契约检查，
//! 构造校验、存储直查、借用归还等内存盘特有判据留在本模块。
//! VirtIO/SD 后端接入时按同一模式建立各自的 tests 模块。

use super::MemoryDisk;
use crate::block::contract::{
    check_block_count, check_buffer_length_validation, check_empty_device,
    check_first_last_write_preserves_neighbors, check_flush_failure_retry, check_flush_ok,
    check_flush_unsupported, check_io_failures_recoverable, check_known_data,
    check_out_of_bounds_rejected, check_read_write_roundtrip,
};
use crate::block::{BLOCK_SIZE, BlockDevice, BlockError, contract};

#[test]
fn contract_read_write_roundtrip() {
    let original: [u8; BLOCK_SIZE] = core::array::from_fn(|i| (i ^ (i >> 8)) as u8);
    let mut storage = original;
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    // 破坏性写入前先确认设备读回镜像已有内容（已知数据读取）。
    check_known_data(&mut disk, 0, &original).unwrap();
    check_read_write_roundtrip(&mut disk).unwrap();
}

#[test]
fn contract_first_last_write_preserves_neighbors() {
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    assert_eq!(disk.block_count(), 3);
    check_first_last_write_preserves_neighbors(&mut disk).unwrap();
}

#[test]
fn contract_out_of_bounds_rejected() {
    let mut storage = [0x33; BLOCK_SIZE * 2];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    // 两块测试盘上契约探针覆盖全部块，等价于全盘直查。
    check_out_of_bounds_rejected(&mut disk).unwrap();
}

#[test]
fn contract_buffer_length_validation() {
    let mut storage = [0x33; BLOCK_SIZE];
    {
        let mut disk = MemoryDisk::new(&mut storage).unwrap();
        check_buffer_length_validation(&mut disk).unwrap();
    }
    // 底层存储保持契约预置的探针基线（probe_pattern(0)），
    // 参数错误的操作未改动盘内任何字节（后端侧直查，契约层看不到存储）。
    let baseline = crate::block::contract::probe_pattern(0);
    assert_eq!(storage, baseline);
}

#[test]
fn rejects_unaligned_capacity_without_modifying_storage() {
    for length in [1, BLOCK_SIZE - 1, BLOCK_SIZE + 1] {
        let mut storage = [0x33; BLOCK_SIZE + 1];
        assert!(matches!(
            MemoryDisk::new(&mut storage[..length]),
            Err(BlockError::InvalidCapacity)
        ));
        assert_eq!(storage, [0x33; BLOCK_SIZE + 1]);
    }
}

#[test]
fn contract_empty_device_has_no_valid_blocks() {
    let mut storage = [];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    check_empty_device(&mut disk).unwrap();
    // 空盘 flush 恒成功是内存盘语义，留在后端侧。
    disk.flush().unwrap();
}

#[test]
fn contract_flush_ok_preserves_data() {
    let mut storage = [0x33; BLOCK_SIZE];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    check_flush_ok(&mut disk).unwrap();
}

#[test]
fn flush_releases_storage_with_written_data() {
    let mut storage = [0x33; BLOCK_SIZE];
    {
        let mut disk = MemoryDisk::new(&mut storage).unwrap();
        disk.write_block(0, &[0x55; BLOCK_SIZE]).unwrap();
        disk.flush().unwrap();
    }
    // 借用归还是内存盘专属判据：释放后写入仍在原缓冲区（HAL-001 容量与借用）。
    assert_eq!(storage, [0x55; BLOCK_SIZE]);
}

#[test]
fn contract_unsupported_sync_reaches_caller() {
    let mut storage = [0; BLOCK_SIZE];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    let device: &mut dyn BlockDevice = &mut disk;
    let mut no_sync = contract::NoSyncDevice::new(device);
    check_flush_unsupported(&mut no_sync).unwrap();
}

#[test]
fn contract_io_failures_reach_caller() {
    let mut storage = [0x33; BLOCK_SIZE];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    let device: &mut dyn BlockDevice = &mut disk;
    let mut faulty = contract::FaultyDevice::new(device);
    check_io_failures_recoverable(&mut faulty).unwrap();
}

#[test]
fn contract_failed_sync_reaches_caller_and_can_be_retried() {
    let mut storage = [0; BLOCK_SIZE];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    let device: &mut dyn BlockDevice = &mut disk;
    let mut faulty = contract::FaultyDevice::new(device);
    check_flush_failure_retry(&mut faulty).unwrap();
}

#[test]
fn contract_run_block_contract_passes_all_scenarios() {
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    contract::run_block_contract("memory-disk", &mut disk, 3).unwrap();
}

#[test]
fn contract_run_block_contract_reports_backend_on_count_mismatch() {
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    assert_eq!(
        contract::run_block_contract("memory-disk", &mut disk, 4),
        Err(contract::BlockContractError {
            backend: "memory-disk",
            failure: contract::ContractFailure {
                scenario: "block_count",
                kind: contract::FailureKind::WrongBlockCount {
                    expected: 4,
                    actual: 3
                }
            }
        })
    );
}

#[test]
fn contract_block_count_matches_expected() {
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    check_block_count(&mut disk, 3).unwrap();
    assert_eq!(
        check_block_count(&mut disk, 4),
        Err(contract::ContractFailure {
            scenario: "block_count",
            kind: contract::FailureKind::WrongBlockCount {
                expected: 4,
                actual: 3
            }
        })
    );
}

#[test]
fn contract_write_isolation_passes() {
    let mut storage = [0x33; BLOCK_SIZE * 3];
    let mut disk = MemoryDisk::new(&mut storage).unwrap();
    contract::check_write_isolation(&mut disk).unwrap();
}
