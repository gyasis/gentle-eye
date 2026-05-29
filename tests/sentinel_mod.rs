use std::path::Path;

#[test]
fn test_dayflow_mod_compiles() {
    // Test that the dayflow module can be compiled and imported
    // This test ensures the module structure is correct and imports work
    assert!(Path::new("src/dayflow/mod.rs").exists());
    
    // Test that all expected re-exports are available
    // This validates the pub use statements in mod.rs
    use dayflow::models::*;
    use dayflow::errors::DayflowError;
    
    // Verify that the types mentioned in the error exist
    let _chunk_ref: Option<ChunkRef> = None;
    let _dayflow_mode: Option<DayflowMode> = None;
    
    // Test that the module can be used without compilation errors
    // This ensures all imports resolve correctly
    let _activity_category: ActivityCategory = ActivityCategory::default();
    let _timeline_entry: TimelineEntry = TimelineEntry::default();
    let _chunk_summary: ChunkSummary = ChunkSummary::default();
    let _dayflow_session: DayflowSession = DayflowSession::default();
    let _dayflow_status: DayflowStatus = DayflowStatus::default();
    let _rolling_context: RollingContext = RollingContext::default();
    
    // Test that error type is available
    let _error: Result<(), DayflowError> = Ok(());
}

#[test]
fn test_dayflow_module_structure() {
    // Test that all expected modules are defined
    use dayflow::daemon;
    use dayflow::engine;
    use dayflow::errors;
    use dayflow::models;
    use dayflow::retention;
    use dayflow::summarizer;
    use dayflow::timeline;
    
    // Verify the modules can be accessed
    assert!(true); // If we get here, modules are accessible
}

#[test]
fn test_dayflow_re_exports() {
    // Test that all re-exported items are accessible
    use dayflow::{
        ActivityCategory, ChunkRef, ChunkSummary, DayflowMode, DayflowSession, 
        DayflowStatus, RollingContext, TimelineEntry, DayflowError
    };
    
    // Verify we can create instances of the types
    let _category = ActivityCategory::default();
    let _summary = ChunkSummary::default();
    let _session = DayflowSession::default();
    let _status = DayflowStatus::default();
    let _context = RollingContext::default();
    let _entry = TimelineEntry::default();
    
    // Test that error can be used
    let _result: Result<(), DayflowError> = Ok(());
}