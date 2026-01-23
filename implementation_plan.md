# Implementation Plan for Telegramic Backup

## 1. Project Scaffolding
- [ ] Create setup script (`setup_project.sh`)
- [ ] Run setup script to initialize Tauri app and dependencies

## 2. Database Schema
- [ ] Define SQL schema (`schema.sql`)
- [ ] Set up `sqlx` migration

## 3. Rust Backend Foundation
- [ ] Define Rust structs in `src-tauri/src/models.rs`
- [ ] Configure `grammers-client` and `tokio`
- [ ] Implement Authentication Commands
- [ ] Implement File System Logic (SQLite "folders", logical paths)
- [ ] Handle `FloodWait` errors

## 4. Frontend Foundation
- [ ] Configure Tailwind CSS with "Midnight Vault" theme
- [ ] Setup `tanstack-query` and `tauri-plugin-store`
- [ ] Create basic layout (Sidebar, Main Grid)
- [ ] Implement routing/navigation

## 5. Feature Implementation
- [ ] **Authentication Flow**: Phone -> Code -> Password -> Dashboard
- [ ] **File Explorer**:
    - [ ] File Grid
    - [ ] Breadcrumbs
    - [ ] Navigation
- [ ] **File Operations**:
    - [ ] Upload (chunking > 2GB)
    - [ ] Download
    - [ ] Create Folder (Virtual)

## 6. Branding
- [ ] Generate App Icon ("The Origami Box")

