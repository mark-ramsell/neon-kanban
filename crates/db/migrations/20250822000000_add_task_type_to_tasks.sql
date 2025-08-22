-- Add task_type column to tasks table for branch prefix functionality
-- This allows tasks to be categorized as feature, bugfix, hotfix, or chore
-- which will be used to prefix git branch names (e.g., feature/vk-abc1-task-name)

ALTER TABLE tasks ADD COLUMN task_type TEXT NOT NULL DEFAULT 'feature' 
    CHECK (task_type IN ('feature', 'bugfix', 'hotfix', 'chore'));