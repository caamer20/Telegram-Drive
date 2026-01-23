Project code name: Telegramic Backup

Project Brief: Rust Telegram File Explorer (TGFS)
Role: You are a Senior Rust Engineer and UI/UX Designer specializing in Tauri v2, Async Rust, and Modern React (Tailwind/Framer Motion).
Goal: Create a polished, self-hosted desktop application that functions as a "File Explorer" for Telegram. It allows users to upload/download files to their "Saved Messages" as if it were a local drive.
1. Tech Stack & Constraints
	•	Core: Rust (Edition 2021)
	•	GUI: Tauri v2 (React + TypeScript + Tailwind CSS).
	•	Telegram Protocol: grammers crate (Pure Rust MTProto). DO NOT use teloxide or frankenstein.
	•	Database: sqlx with SQLite (stored in $APP_DATA).
	•	State Management: tanstack-query (React) + tauri-plugin-store (Persistence).
	•	Icons: lucide-react.
2. Architecture Overview
The app acts as a virtual filesystem.
	•	Frontend: A responsive "Finder/Explorer" view. Handles drag-and-drop, file previews, and folder navigation.
	•	Backend (Rust):
	•	Connection: Persistent grammers::Client.
	•	Virtual FS: Uses SQLite to create a "Folder" structure (since Telegram is flat).
	•	Chunking: Splits files >2GB into chunks, uploads them, and links the Message IDs in SQLite.
3. The Login Service (Auth Flow)
The application must be self-contained (User uses their own phone number).
Backend (Rust) Commands
Expose these Tauri commands:
	0.	cmd_auth_request_code(phone: String) -> Returns phone_code_hash.
	0.	cmd_auth_sign_in(code: String, hash: String) -> Returns Success or PasswordRequired.
	0.	cmd_auth_check_password(password: String) -> Finishes login (2FA).
Frontend (React)
Implement a multi-step wizard: PhoneInput -> SmsCodeInput -> 2FAPassword (if needed) -> Dashboard.

4. Frontend Design System ("The Inverse Telegram")
The UI should feel familiar to Telegram users but distinct enough to signify "Storage" rather than "Chat".
Color Palette: "The Midnight Vault"
We invert Telegram's standard "Light Blue & White" theme into a high-contrast "Deep Navy & Gold" theme.
	•	Background: Deep Midnight (#0e1621) - Use this instead of Telegram's white.
	•	Surface/Cards: Lighter Navy (#17212b) - For the file grid and sidebar.
	•	Primary Accent: Data Amber (#ffae00) - Used for primary buttons (Upload, Connect) and folder icons. This is the complementary "inverse" of Telegram Blue.
	•	Secondary Accent: Telegram Blue (#2481cc) - Used sparingly for status indicators (e.g., "Syncing", "Connected") to pay homage to the underlying protocol.
	•	Text: Crisp White (#ffffff) for headers; Steel Grey (#8e9fb3) for metadata.
UI Components
	•	Sidebar: Dark Navy. Lists "Drives" (different chats/channels used for storage).
	•	Main Grid: Uses a CSS Grid layout. Files are represented as Cards.
	•	Hover State: Cards lift slightly (transform: translateY) and glow with a subtle Amber border.
	•	Progress Bar: When uploading, show a thin Amber line at the very top of the window (like iOS Safari) rather than a popup modal.

5. Branding & Logo Concept
The logo needs to convey "Telegram" + "Storage" + "Self-Hosted".
The Concept: "The Origami Box"
Telegram's logo is a Paper Plane. Our logo transforms that paper metaphor into a Cardboard Box.
	•	Shape: An isometric open box (cube), formed by folded paper planes. It looks like a storage container but constructed from the "material" of Telegram.
	•	Iconography:
	•	The "flaps" of the box evoke the wings of the paper plane.
	•	Inside the box, a subtle "lock" symbol or a "hard drive" platter suggests secure, self-hosted data.
	•	Coloring:
	•	Outer Box: A gradient of Telegram Blue (#2481cc) to Deep Cyan.
	•	Inner Box: A glowing Amber (#ffae00) interior, representing the precious data stored inside.
SVG Implementation (Instructions for Designer/AI)
Create an SVG with:
	0.	viewBox="0 0 512 512".
	0.	Base: A rounded hexagon (isometric cube silhouette).
	0.	Path 1 (Left Face): Darker Blue shade.
	0.	Path 2 (Right Face): Lighter Blue shade.
	0.	Path 3 (Top/Inside): An open flap revealing a gold/amber gradient gradient.
	0.	Detail: A stylized white "Rust Gear" hidden subtly on the front face of the box.
This logo should be exported as app-icon.svg and favicon.ico.

6. Execution Plan (Antigravity Instructions)
Please generate the following Artifacts before writing code:
	0.	Project Scaffold Script: A bash script to run npm create tauri-app, install grammers-client, tokio, sqlx, lucide-react, and framer-motion.
	0.	Database Schema: The SQL implementation for CREATE TABLE files.
	0.	Rust Type Definitions: The structs for AuthState and FileMetadata.
	0.	Component Tree: A list of React components needed (e.g., FileGrid, Breadcrumbs, UploadZone).
Critical Instruction: Ensure the Rust backend handles the GlobalResult::FloodWait error from Telegram. If we hit a rate limit, the backend must sleep automatically or notify the frontend, rather than crashing.
