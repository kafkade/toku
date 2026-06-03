-- Track which device performed a soft delete.
ALTER TABLE books ADD COLUMN deleted_by_device TEXT;
