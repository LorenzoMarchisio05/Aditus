import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import Palette from './Palette.svelte';
import Panel from './Panel.svelte';

const view = new URLSearchParams(location.search).get('view');
const Root = view === 'palette' ? Palette : view ? Panel : App;
mount(Root, { target: document.getElementById('app'), props: { view } });
