window.SIDEBAR_ITEMS = {"enum":[["RemoteFile",""]],"fn":[["resolve_path","Resolve a path that might contain symlinks from a specific container to a path accessible from the root host"]],"struct":[["FileManager",""]],"type":[["GetDEnts64Stream","`Peekable`: So that we can stop consuming if there is no more place in buf. `Chain`: because `read_dir`’s returned stream does not contain `.` and `..`. So we chain our own stream with `.` and `..` in it to the one returned by `read_dir`. `IntoIter`: That’s our DIY stream with `.` and `..` ^. first `Map`: Converting into DirEntryInternal. second `Map`: logging any errors from the first map."]]};