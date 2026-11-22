package dev.fetchx.downloads.settings

data class CategoryRule(
    val folder: String,
    val extensions: String,
)

data class FolderCategoriesConfig(
    val enabled: Boolean = false,
    val compressed: CategoryRule = CategoryRule("Compressed", "zip,rar,7z,gz,gzip,tar,tgz,bz2,xz,z,cab,arj,lzh,lha,ace,sit,sitx,jar"),
    val documents: CategoryRule = CategoryRule("Documents", "pdf,doc,docx,xls,xlsx,ppt,pptx,txt,rtf,odt,ods,odp,csv,epub,chm"),
    val music: CategoryRule = CategoryRule("Music", "mp3,wav,wma,ogg,flac,aac,m4a,mid,midi,aif,aiff,ra,ape,opus"),
    val programs: CategoryRule = CategoryRule("Programs", "exe,msi,dmg,pkg,deb,rpm,apk,appimage,bin,iso,bat"),
    val video: CategoryRule = CategoryRule("Video", "mp4,mkv,avi,mov,wmv,flv,webm,mpg,mpeg,m4v,3gp,ts,vob,asf,rm,ogv"),
    val images: CategoryRule = CategoryRule("Images", "jpg,jpeg,png,gif,bmp,webp,svg,ico,tiff,tif,heic,heif,raw,cr2,nef"),
    val custom: String = "",
)
