use crate::protocol::{Task, TaskStatus};

pub struct GovernorReport {
    pub failed_tasks: Vec<Task>,
    pub all_successful: bool,
}

pub fn review_execution(tasks: &Vec<Task>) -> GovernorReport {
    println!("\n⚖️  Governor: Reviewing execution results...");
    
    let failed_tasks: Vec<Task> = tasks.iter()
        .filter(|t| matches!(t.status, TaskStatus::Failed))
        .cloned()
        .collect();

    let completed_count = tasks.iter()
        .filter(|t| matches!(t.status, TaskStatus::Completed))
        .count();

    let all_successful = failed_tasks.is_empty() && completed_count == tasks.len();

    if all_successful {
        println!("  ✅ All tasks validated successfully.");
    } else {
        println!("  📊 Status: {}/{} completed, {} failed", completed_count, tasks.len(), failed_tasks.len());
        for task in &failed_tasks {
            println!("  ⚠️  Task '{}' FAILED. Marking for retry.", task.intent);
        }
    }

    GovernorReport {
        failed_tasks,
        all_successful,
    }
}

pub fn reset_failed_tasks(tasks: &mut Vec<Task>) {
    for task in tasks.iter_mut().filter(|t| matches!(t.status, TaskStatus::Failed)) {
        task.status = TaskStatus::Pending; // Set back to Pending for Dispatcher to see
    }
}