export interface UiGroup {id:string;name:string;members:string[]}
export interface SearchSettings {enter:'primary'|'details';match_path:boolean;show_actions:boolean}
export interface Preferences {theme:string;groups:UiGroup[];collapsed:string[];show_recent:boolean;last_used:Record<string,number>;search:SearchSettings}
export const defaults = ():Preferences => ({theme:'system',groups:[],collapsed:[],show_recent:true,last_used:{},search:{enter:'primary',match_path:false,show_actions:true}});
