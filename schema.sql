-- Virtual Filesystem Structure

-- Folders: Virtual directory structure
CREATE TABLE IF NOT EXISTS folders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id INTEGER,
    name TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(parent_id) REFERENCES folders(id) ON DELETE CASCADE
);

-- Files: Metadata for files stored in Telegram
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    folder_id INTEGER,
    name TEXT NOT NULL,
    size INTEGER NOT NULL, -- Size in bytes
    mime_type TEXT,
    is_chunked BOOLEAN DEFAULT 0,
    
    -- Telegram Linkage
    chat_id INTEGER NOT NULL, -- The ID of the chat/channel where file is stored
    message_id INTEGER,       -- The message ID containing the file (or first chunk)
    
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(folder_id) REFERENCES folders(id) ON DELETE CASCADE
);

-- File Chunks: For files > 2GB (Telegram Limit) or split for other reasons
CREATE TABLE IF NOT EXISTS file_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL,
    chunk_index INTEGER NOT NULL, -- 0-based index
    telegram_message_id INTEGER NOT NULL,
    chunk_size INTEGER NOT NULL,
    
    FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
);

-- Drives / Mounts: Mapping friendly names to Chat IDs
CREATE TABLE IF NOT EXISTS drives (
    chat_id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    icon TEXT, -- Lucide icon name or emoji
    is_active BOOLEAN DEFAULT 1
);
