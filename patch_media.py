import re

with open("app/src-tauri/src/migration/adapters/media.rs", "r") as f:
    content = f.read()

# Make the changes in media.rs to support HDR detection and proper decision logic.
# I will use a python script because the changes might be extensive and multi_replace_file_content is better if I can write a precise Python script.
