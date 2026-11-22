use serde::{Deserialize, Serialize};

/// Configuration for automatic folder categorisation of finished downloads.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct FolderCategoriesConfig {
    pub enabled: bool,
    pub compressed: CategoryRule,
    pub documents: CategoryRule,
    pub music: CategoryRule,
    pub programs: CategoryRule,
    pub video: CategoryRule,
    pub images: CategoryRule,
    /// Extra rules in "Folder=ext,ext;Folder2=ext" format, same as the motrix plugin.
    pub custom: String,
}

impl Default for FolderCategoriesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            compressed: CategoryRule {
                folder: "Compressed".to_owned(),
                extensions: "zip,rar,7z,gz,gzip,tar,tgz,bz2,xz,z,cab,arj,lzh,lha,ace,sit,sitx,jar"
                    .to_owned(),
            },
            documents: CategoryRule {
                folder: "Documents".to_owned(),
                extensions: "pdf,doc,docx,xls,xlsx,ppt,pptx,txt,rtf,odt,ods,odp,csv,epub,chm"
                    .to_owned(),
            },
            music: CategoryRule {
                folder: "Music".to_owned(),
                extensions: "mp3,wav,wma,ogg,flac,aac,m4a,mid,midi,aif,aiff,ra,ape,opus".to_owned(),
            },
            programs: CategoryRule {
                folder: "Programs".to_owned(),
                extensions: "exe,msi,dmg,pkg,deb,rpm,apk,appimage,bin,iso,bat".to_owned(),
            },
            video: CategoryRule {
                folder: "Video".to_owned(),
                extensions: "mp4,mkv,avi,mov,wmv,flv,webm,mpg,mpeg,m4v,3gp,ts,vob,asf,rm,ogv"
                    .to_owned(),
            },
            images: CategoryRule {
                folder: "Images".to_owned(),
                extensions: "jpg,jpeg,png,gif,bmp,webp,svg,ico,tiff,tif,heic,heif,raw,cr2,nef"
                    .to_owned(),
            },
            custom: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CategoryRule {
    /// Folder name (relative = created inside download dir) or absolute path.
    pub folder: String,
    /// Comma-separated extensions without a leading dot.
    pub extensions: String,
}

impl Default for CategoryRule {
    fn default() -> Self {
        Self {
            folder: String::new(),
            extensions: String::new(),
        }
    }
}
