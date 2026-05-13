use std::path::Path;
use tailtalk_packets::afp::AfpError;
use tracing::error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IconKey {
    pub creator: [u8; 4],
    pub file_type: [u8; 4],
    pub icon_type: u8,
}

impl IconKey {
    pub fn to_bytes(&self) -> [u8; 9] {
        let mut bytes = [0u8; 9];
        // AFP creator and file_type are exactly 4 bytes (Mac OS OSType format)
        bytes[0..4].copy_from_slice(&self.creator);
        bytes[4..8].copy_from_slice(&self.file_type);

        bytes[8] = self.icon_type;
        bytes
    }
}

pub struct DesktopDatabase {
    pub dt_ref_num: u16,
    db: sled::Db,
}

impl DesktopDatabase {
    pub fn new(volume_root: &Path, dt_ref_num: u16) -> Result<Self, AfpError> {
        let db_path = volume_root.join(".tailtalk").join("desktop.db");

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                error!(
                    "Failed to create Desktop Database directory at {:?}: {}",
                    parent, e
                );
                AfpError::AccessDenied
            })?;
        }

        let db = sled::open(&db_path).map_err(|e| {
            error!("Failed to open Desktop Database at {:?}: {}", db_path, e);
            AfpError::AccessDenied
        })?;

        Ok(Self { dt_ref_num, db })
    }

    pub fn add_icon(
        &self,
        creator: [u8; 4],
        file_type: [u8; 4],
        icon_type: u8,
        icon_data: &[u8],
    ) -> Result<(), AfpError> {
        let key = IconKey {
            creator,
            file_type,
            icon_type,
        };

        let tree = self.db.open_tree(b"icons").map_err(|e| {
            error!("Failed to open 'icons' tree: {}", e);
            AfpError::AccessDenied
        })?;

        tree.insert(key.to_bytes(), icon_data).map_err(|e| {
            error!("Failed to insert icon: {}", e);
            AfpError::AccessDenied
        })?;
        Ok(())
    }

    pub fn get_icon(
        &self,
        creator: [u8; 4],
        file_type: [u8; 4],
        icon_type: u8,
        _size: u16,
    ) -> Result<Vec<u8>, AfpError> {
        let key = IconKey {
            creator,
            file_type,
            icon_type,
        };

        let tree = self.db.open_tree(b"icons").map_err(|e| {
            error!("Failed to open 'icons' tree: {}", e);
            AfpError::AccessDenied
        })?;

        if let Some(data) = tree.get(key.to_bytes()).map_err(|e| {
            error!("Failed to get icon: {}", e);
            AfpError::AccessDenied
        })? {
            Ok(data.to_vec())
        } else {
            Err(AfpError::ItemNotFound)
        }
    }
    pub fn get_icon_info(
        &self,
        creator: [u8; 4],
        _icon_type: u16,
    ) -> Result<(u32, u32, u16), AfpError> {
        // Since sqlite/sled stores the entire icon, we can iterate over the keys matching the creator
        // and find an icon type that matches the request. Or return basic size info.
        let tree = self.db.open_tree(b"icons").map_err(|e| {
            error!("Failed to open 'icons' tree: {}", e);
            AfpError::AccessDenied
        })?;

        // Format is:
        // tag (4 bytes)
        // file_type (4 bytes)
        // icon_type (1 byte, padding, or matching requested size)
        // size (2 bytes)

        for result in tree.iter() {
            if let Ok((key, value)) = result
                && key.len() == 9
            {
                let mut key_creator = [0u8; 4];
                key_creator.copy_from_slice(&key[0..4]);
                if key_creator == creator {
                    let mut file_type = [0u8; 4];
                    file_type.copy_from_slice(&key[4..8]);

                    // We just return the first icon we find for this creator
                    // For a full implementation we would want to correctly parse the icon_type u16 request into the actual icon_type u8
                    // Return: IconTag (4 bytes), FileCreator/Type (4 bytes), Size (2 bytes)
                    let icon_tag = 0; // Or whatever tag you want
                    let file_type_u32 = u32::from_be_bytes(file_type);
                    let size = value.len() as u16;

                    return Ok((icon_tag, file_type_u32, size));
                }
            }
        }

        Err(AfpError::ItemNotFound)
    }

    pub fn set_comment(&self, node_id: u32, comment: &[u8]) -> Result<(), AfpError> {
        let tree = self.db.open_tree(b"comments").map_err(|e| {
            error!("Failed to open 'comments' tree: {}", e);
            AfpError::AccessDenied
        })?;

        tree.insert(node_id.to_be_bytes(), comment).map_err(|e| {
            error!("Failed to insert comment: {}", e);
            AfpError::AccessDenied
        })?;

        tracing::info!(
            "Set comment of length {} for node {}",
            comment.len(),
            node_id
        );
        Ok(())
    }

    pub fn get_comment(&self, node_id: u32) -> Result<Vec<u8>, AfpError> {
        let tree = self.db.open_tree(b"comments").map_err(|e| {
            error!("Failed to open 'comments' tree: {}", e);
            AfpError::AccessDenied
        })?;

        tracing::info!("Get comment for node {}", node_id);
        if let Some(data) = tree.get(node_id.to_be_bytes()).map_err(|e| {
            error!("Failed to get comment: {}", e);
            AfpError::AccessDenied
        })? {
            Ok(data.to_vec())
        } else {
            Err(AfpError::ItemNotFound)
        }
    }

    pub fn remove_comment(&self, node_id: u32) -> Result<(), AfpError> {
        let tree = self.db.open_tree(b"comments").map_err(|e| {
            error!("Failed to open 'comments' tree: {}", e);
            AfpError::AccessDenied
        })?;

        tree.remove(node_id.to_be_bytes()).map_err(|e| {
            error!("Failed to remove comment: {}", e);
            AfpError::AccessDenied
        })?;
        Ok(())
    }

    /// Register an application in the Desktop DB.
    ///
    /// Key: creator (4) + tag (4, big-endian) — one entry per (creator, tag) pair.
    /// Value: directory_id (4) + path_len (1) + path bytes.
    pub fn add_appl(
        &self,
        creator: [u8; 4],
        tag: u32,
        directory_id: u32,
        path: &str,
    ) -> Result<(), AfpError> {
        let tree = self.db.open_tree(b"appls").map_err(|e| {
            error!("Failed to open 'appls' tree: {}", e);
            AfpError::AccessDenied
        })?;

        let mut key = [0u8; 8];
        key[0..4].copy_from_slice(&creator);
        key[4..8].copy_from_slice(&tag.to_be_bytes());

        let path_bytes = path.as_bytes();
        let mut value = Vec::with_capacity(5 + path_bytes.len());
        value.extend_from_slice(&directory_id.to_be_bytes());
        value.push(path_bytes.len() as u8);
        value.extend_from_slice(path_bytes);

        tree.insert(key, value).map_err(|e| {
            error!("Failed to insert APPL: {}", e);
            AfpError::AccessDenied
        })?;
        Ok(())
    }

    /// Deregister an application identified by (creator, directory_id, path).
    pub fn remove_appl(
        &self,
        creator: [u8; 4],
        directory_id: u32,
        path: &str,
    ) -> Result<(), AfpError> {
        let tree = self.db.open_tree(b"appls").map_err(|e| {
            error!("Failed to open 'appls' tree: {}", e);
            AfpError::AccessDenied
        })?;

        let dir_bytes = directory_id.to_be_bytes();
        let path_bytes = path.as_bytes();

        for result in tree.scan_prefix(creator) {
            let (key, value) = result.map_err(|e| {
                error!("Failed to scan APPLs: {}", e);
                AfpError::AccessDenied
            })?;

            if value.len() >= 5 {
                let path_len = value[4] as usize;
                if &value[0..4] == dir_bytes
                    && value.len() >= 5 + path_len
                    && &value[5..5 + path_len] == path_bytes
                {
                    tree.remove(&key).map_err(|e| {
                        error!("Failed to remove APPL: {}", e);
                        AfpError::AccessDenied
                    })?;
                    return Ok(());
                }
            }
        }

        Err(AfpError::ItemNotFound)
    }

    /// Retrieve a registered application by creator and 1-based index.
    ///
    /// Returns (tag, directory_id, path).
    pub fn get_appl(
        &self,
        creator: [u8; 4],
        index: u16,
    ) -> Result<(u32, u32, String), AfpError> {
        let tree = self.db.open_tree(b"appls").map_err(|e| {
            error!("Failed to open 'appls' tree: {}", e);
            AfpError::AccessDenied
        })?;

        let target = index.saturating_sub(1) as usize;
        for (i, result) in tree.scan_prefix(creator).enumerate() {
            let (key, value) = result.map_err(|e| {
                error!("Failed to scan APPLs: {}", e);
                AfpError::AccessDenied
            })?;

            if i == target {
                if key.len() < 8 || value.len() < 5 {
                    return Err(AfpError::ItemNotFound);
                }
                let tag = u32::from_be_bytes(key[4..8].try_into().unwrap());
                let directory_id = u32::from_be_bytes(value[0..4].try_into().unwrap());
                let path_len = value[4] as usize;
                let path = if value.len() >= 5 + path_len {
                    String::from_utf8_lossy(&value[5..5 + path_len]).into_owned()
                } else {
                    String::new()
                };
                return Ok((tag, directory_id, path));
            }
        }

        Err(AfpError::ItemNotFound)
    }
}
